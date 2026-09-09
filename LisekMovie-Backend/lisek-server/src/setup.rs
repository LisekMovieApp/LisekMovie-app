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
    let mode = state.mode.read().await;
    let (setup_required, database_connected) = match &*mode {
        ServerMode::SetupRequired => (true, false),
        ServerMode::DatabaseError(_) => (false, false),
        ServerMode::Ready(_) => (false, true),
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

/// Refuses to run any setup endpoint once a working database is already
/// configured — otherwise anyone could hit `/api/setup/finish` after the
/// fact and create a second, unauthenticated admin account.
async fn guard_setup_not_already_done(state: &AppState) -> Result<(), SetupError> {
    let mode = state.mode.read().await;
    if !matches!(&*mode, ServerMode::SetupRequired) {
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

    lisek_db::connect_with_config(&config)
        .await
        .map_err(db_error_to_setup_error)?;

    Ok(Json(TestConnectionResponse { ok: true }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct FinishRequest {
    pub database: DatabaseConfig,
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

    let pool = lisek_db::connect_with_config(&req.database)
        .await
        .map_err(db_error_to_setup_error)?;

    lisek_db::run_migrations(&pool)
        .await
        .map_err(db_error_to_setup_error)?;

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

    if let Err(err) = crate::env_file::save_database_url(&req.database.to_connection_url()) {
        tracing::error!("failed to persist DATABASE_URL to .env: {err}");
        return Err(setup_error(
            "config_write_failed",
            "The database was set up, but saving the configuration to disk failed. \
             Setting DATABASE_URL yourself and restarting the server will fix this.",
        ));
    }

    *state.mode.write().await = ServerMode::Ready(pool);

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
