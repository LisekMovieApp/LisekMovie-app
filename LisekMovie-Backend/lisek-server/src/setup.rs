//! First-Run Wizard endpoints: check status, test a database connection,
//! and finish setup by creating the first admin account.
//!
//! Every handler returns `error_code` (stable, English) rather than a
//! translated `message` — translation happens on the frontend, from the
//! same i18n dictionary as the rest of the UI, keyed by `error_code`. See
//! the architecture plan's error-shape rule. `message` is still filled in
//! with a plain English description, useful for logs and for any client
//! that hasn't got a translation for a given code yet.

use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use lisek_db::{DatabaseConfig, DbError};

use crate::state::{AppState, ServerMode, SharedState};

#[derive(Serialize, utoipa::ToSchema)]
pub struct StatusResponse {
    pub setup_required: bool,
    pub database_connected: bool,
}

/// Always reachable, in every server mode — this is the first thing the
/// frontend calls on every page load to decide whether to redirect to
/// `/setup`.
#[utoipa::path(get,path="/api/status",tag="setup",responses((status=200,body=StatusResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)))]
pub async fn status(State(state): State<SharedState>) -> Json<StatusResponse> {
    let mode = state.mode.load();
    let (setup_required, database_connected) = match &**mode {
        ServerMode::SetupRequired => (true, false),
        ServerMode::DatabaseError(_) => (false, false),
        // A database from `DATABASE_URL` without any account yet: the
        // wizard skips its database step and only creates the admin.
        ServerMode::Ready(_) => (pool_without_accounts(&state).await.is_some(), true),
    };
    Json(StatusResponse {
        setup_required,
        database_connected,
    })
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SetupError {
    pub error_code: &'static str,
    pub message: String,
}

impl IntoResponse for SetupError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

fn setup_error(error_code: &'static str, message: impl Into<String>) -> SetupError {
    SetupError {
        error_code,
        message: message.into(),
    }
}

/// The connected database when it was provided through `DATABASE_URL`
/// (Docker) and no account exists yet; `None` otherwise. Once an account
/// is seen the answer is cached: accounts are never all deleted.
async fn pool_without_accounts(state: &AppState) -> Option<lisek_db::PgPool> {
    use std::sync::atomic::Ordering;
    if state.has_accounts.load(Ordering::Relaxed) {
        return None;
    }
    let pool = state.pool()?;
    match lisek_db::users::any_user_exists(&pool).await {
        Ok(false) => Some(pool),
        Ok(true) => {
            state.has_accounts.store(true, Ordering::Relaxed);
            None
        }
        Err(_) => None,
    }
}

/// Refuses to run any setup endpoint once setup is done — otherwise anyone
/// could hit `/api/setup/finish` after the fact and create a second,
/// unauthenticated admin account. Setup is open while no database is
/// configured, or while a preconfigured one has no account yet.
async fn guard_setup_not_already_done(state: &AppState) -> Result<(), SetupError> {
    let open = match &**state.mode.load() {
        ServerMode::SetupRequired => true,
        ServerMode::Ready(_) => pool_without_accounts(state).await.is_some(),
        ServerMode::DatabaseError(_) => false,
    };
    if !open {
        return Err(setup_error(
            "setup_already_complete",
            "Setup has already been completed on this server.",
        ));
    }
    Ok(())
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TestConnectionResponse {
    pub ok: bool,
}

#[utoipa::path(post,path="/api/setup/database/test",tag="setup",request_body=DatabaseConfig,responses((status=200,body=TestConnectionResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)))]
pub async fn test_database(
    State(state): State<SharedState>,
    Json(config): Json<DatabaseConfig>,
) -> Result<Json<TestConnectionResponse>, SetupError> {
    guard_setup_not_already_done(&state).await?;
    if !matches!(&**state.mode.load(), ServerMode::SetupRequired) {
        return Err(setup_error("setup_already_complete", "The database is already configured."));
    }

    lisek_db::connect_with_config(&config)
        .await
        .map_err(db_error_to_setup_error)?;

    Ok(Json(TestConnectionResponse { ok: true }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct FinishRequest {
    /// Required unless the server already has a database (`DATABASE_URL`).
    #[serde(default)]
    pub database: Option<DatabaseConfig>,
    pub admin_username: String,
    pub admin_password: String,
    pub ui_language: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct FinishResponse {
    pub ok: bool,
}

#[utoipa::path(post,path="/api/setup/finish",tag="setup",request_body=FinishRequest,responses((status=200,body=FinishResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)))]
pub async fn finish(
    State(state): State<SharedState>,
    Json(req): Json<FinishRequest>,
) -> Result<Json<FinishResponse>, SetupError> {
    static SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SETUP_LOCK.lock().await;
    guard_setup_not_already_done(&state).await?;

    if !matches!(req.ui_language.as_str(), "en" | "pl") || req.admin_username.len() > 128 || req.admin_password.len() > 1024 {
        return Err(setup_error("invalid_request", "Invalid account settings."));
    }

    if req.admin_username.trim().is_empty() {
        return Err(setup_error(
            "invalid_request",
            "Admin username cannot be empty.",
        ));
    }

    // A database from `DATABASE_URL` is already connected and migrated.
    let preconfigured = pool_without_accounts(&state).await;
    let database = match (&preconfigured, req.database.as_ref()) {
        (Some(_), _) => None,
        (None, Some(database)) => Some(database),
        (None, None) => return Err(setup_error("invalid_request", "Database settings are missing.")),
    };
    let pool = match (&preconfigured, database) {
        (Some(pool), _) => pool.clone(),
        (None, Some(database)) => {
            let pool = lisek_db::connect_with_config(database)
                .await
                .map_err(db_error_to_setup_error)?;
            lisek_db::run_migrations(&pool)
                .await
                .map_err(db_error_to_setup_error)?;
            pool
        }
        (None, None) => unreachable!("checked above"),
    };

    let password_hash =
        lisek_auth::hash_password(&req.admin_password).map_err(|err| match err {
            lisek_auth::PasswordError::TooShort => setup_error(
                "password_too_weak",
                format!(
                    "Password must be at least {} characters long.",
                    lisek_auth::MIN_PASSWORD_LENGTH
                ),
            ),
            _ => setup_error("internal_error", "Could not process the password."),
        })?;

    lisek_db::insert_admin_user(&pool, req.admin_username.trim(), &password_hash, &req.ui_language)
        .await
        .map_err(db_error_to_setup_error)?;

    state.has_accounts.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(database) = database {
        if let Err(err) = crate::env_file::save_database_url(&database.to_connection_url()) {
            tracing::error!("failed to persist DATABASE_URL to .env: {err}");
            return Err(setup_error(
                "config_write_failed",
                "The database was set up, but saving the configuration to disk failed. \
                 Setting DATABASE_URL yourself and restarting the server will fix this.",
            ));
        }
        state.set_mode(ServerMode::Ready(pool));
    }
    // A scan cannot have been queued before setup, but the worker sleeps
    // until woken; this makes it pick up the database right away.
    state.scan_wakeup.notify_one();
    state.trickplay_wakeup.notify_one();

    tracing::info!("First-Run Wizard completed — server is now fully configured.");

    Ok(Json(FinishResponse { ok: true }))
}

fn db_error_to_setup_error(err: DbError) -> SetupError {
    match err {
        DbError::ConnectionFailed(details) => setup_error(
            "database_connection_failed",
            format!("Could not connect to the database: {details}"),
        ),
        DbError::ConnectionTimeout => setup_error(
            "database_connection_timeout",
            "Connecting to the database timed out.",
        ),
        DbError::MigrationFailed(details) => setup_error(
            "migration_failed",
            format!("Database migrations failed: {details}"),
        ),
        DbError::UsernameTaken => setup_error("username_taken", "That username is already taken."),
        // Every other variant (provider-config errors, Faza 5's library
        // errors, ...) is never actually produced by anything the wizard
        // calls — a genuine wildcard here, rather than matching each one by
        // name, is what lets a future variant get added to `DbError`
        // without this match needing an update too.
        _ => setup_error("database_error", err.to_string()),
    }
}
