//! PostgreSQL access layer (sqlx) and migrations.
//!
//! Faza 2 added connecting (discrete fields or a saved URL), running
//! migrations, and creating the first admin account. Faza 3 adds session
//! storage/lookup for the login flow. Everything else (libraries, media
//! items, ...) is added as later phases need it.

use std::time::Duration;

use chrono::{DateTime, Utc};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
pub use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use thiserror::Error;
use uuid::Uuid;

use lisek_core::User;

/// The raw shape every `users` row comes back as — used both for the plain
/// `User` domain type and (with `password_hash` on the side) for login,
/// which needs the hash but must never let it leak into `User` itself.
type UserRow = (Uuid, String, bool, String, Option<String>, DateTime<Utc>);

fn user_from_row(row: UserRow) -> User {
    User {
        id: row.0,
        username: row.1,
        is_admin: row.2,
        ui_language: row.3,
        email: row.4,
        created_at: row.5,
    }
}

/// How long we're willing to wait for a database to respond before giving
/// up — both during the wizard's "test connection" step and at normal
/// startup. Long enough for a slow local Postgres to wake up, short enough
/// that a wrong host doesn't leave the admin staring at a spinner forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum DbError {
    #[error("could not connect to the database: {0}")]
    ConnectionFailed(String),
    #[error("connecting to the database timed out")]
    ConnectionTimeout,
    #[error("failed to run database migrations: {0}")]
    MigrationFailed(String),
    #[error("that username is already taken")]
    UsernameTaken,
    #[error("database error: {0}")]
    Other(String),
}

impl From<sqlx::Error> for DbError {
    fn from(err: sqlx::Error) -> Self {
        // Postgres unique_violation is SQLSTATE 23505 — this is how a
        // duplicate username surfaces from the INSERT below.
        if let Some(db_err) = err.as_database_error()
            && db_err.code().as_deref() == Some("23505")
        {
            return DbError::UsernameTaken;
        }
        DbError::Other(err.to_string())
    }
}

/// Discrete connection fields, as collected by the First-Run Wizard form.
/// Kept separate from a raw connection-string type so the wizard never has
/// to build (or the frontend parse) a `postgres://` URL by hand.
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
}

impl DatabaseConfig {
    /// A `postgres://` URL suitable for persisting as `DATABASE_URL` in
    /// `.env`, so a later plain `connect()` can reconnect without needing
    /// the discrete fields again. Username/password are percent-encoded so
    /// special characters in either don't break the URL.
    pub fn to_connection_url(&self) -> String {
        let user = utf8_percent_encode(&self.username, NON_ALPHANUMERIC);
        let pass = utf8_percent_encode(&self.password, NON_ALPHANUMERIC);
        format!(
            "postgres://{user}:{pass}@{}:{}/{}",
            self.host, self.port, self.database
        )
    }

    fn to_connect_options(&self) -> PgConnectOptions {
        PgConnectOptions::new()
            .host(&self.host)
            .port(self.port)
            .database(&self.database)
            .username(&self.username)
            .password(&self.password)
    }
}

/// Connects using discrete fields — used by both the wizard's "test
/// connection" step and its final "finish" step.
pub async fn connect_with_config(config: &DatabaseConfig) -> Result<PgPool, DbError> {
    connect_with_timeout(PgPoolOptions::new().connect_with(config.to_connect_options())).await
}

/// Connects using a full `postgres://` URL — used at normal server startup,
/// once a connection has already been saved from a completed wizard run.
pub async fn connect(database_url: &str) -> Result<PgPool, DbError> {
    connect_with_timeout(PgPoolOptions::new().connect(database_url)).await
}

async fn connect_with_timeout<F>(connect_future: F) -> Result<PgPool, DbError>
where
    F: std::future::Future<Output = Result<PgPool, sqlx::Error>>,
{
    match tokio::time::timeout(CONNECT_TIMEOUT, connect_future).await {
        Ok(Ok(pool)) => Ok(pool),
        Ok(Err(err)) => Err(DbError::ConnectionFailed(err.to_string())),
        Err(_elapsed) => Err(DbError::ConnectionTimeout),
    }
}

/// Applies every migration in `migrations/` that hasn't already run.
/// Embedded at compile time — no filesystem access needed at runtime.
pub async fn run_migrations(pool: &PgPool) -> Result<(), DbError> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .map_err(|err| DbError::MigrationFailed(err.to_string()))
}

/// Creates the first admin account. Only ever called once, from the
/// wizard's "finish" step — nothing currently re-checks that no admin
/// already exists, because that guard lives at the setup-mode level in
/// lisek-server (setup endpoints refuse to run at all once a connection is
/// already configured).
pub async fn insert_admin_user(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
    ui_language: &str,
) -> Result<User, DbError> {
    let row = sqlx::query_as::<_, UserRow>(
        r#"
        INSERT INTO users (username, password_hash, is_admin, ui_language)
        VALUES ($1, $2, true, $3)
        RETURNING id, username, is_admin, ui_language, email, created_at
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(ui_language)
    .fetch_one(pool)
    .await?;

    Ok(user_from_row(row))
}

/// Looks up a user by username for login — returns the domain `User`
/// alongside the password hash as a separate value, deliberately never
/// attached to `User` itself (see [`lisek_core::User`]'s own doc comment
/// and its "never serializes a password" test). `None` means no such user;
/// the caller (lisek-server's login handler) is responsible for still
/// running a dummy password verification in that case, so a request for a
/// nonexistent username doesn't respond measurably faster than a wrong
/// password for a real one.
#[allow(clippy::type_complexity)]
pub async fn find_user_with_password_hash(
    pool: &PgPool,
    username: &str,
) -> Result<Option<(User, String)>, DbError> {
    let row = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            bool,
            String,
            Option<String>,
            DateTime<Utc>,
            String,
        ),
    >(
        r#"
        SELECT id, username, is_admin, ui_language, email, created_at, password_hash
        FROM users
        WHERE username = $1
        "#,
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(id, username, is_admin, ui_language, email, created_at, password_hash)| {
            (
                user_from_row((id, username, is_admin, ui_language, email, created_at)),
                password_hash,
            )
        },
    ))
}

/// Records a new login session. `token_hash` must already be hashed (see
/// `lisek_auth::hash_session_token`) — this function never sees, and the
/// `sessions` table never stores, the raw token.
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    client: &str,
    device: &str,
    device_id: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO sessions (user_id, token_hash, client, device, device_id, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(client)
    .bind(device)
    .bind(device_id)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// The per-request auth check: resolves an already-hashed session token to
/// the user it belongs to, or `None` if there is no such session, or it has
/// expired. This is the one query `AuthUser` (in lisek-server) runs on every
/// protected request.
pub async fn find_user_by_session_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT u.id, u.username, u.is_admin, u.ui_language, u.email, u.created_at
        FROM sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1 AND s.expires_at > now()
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(user_from_row))
}

/// Deletes a session by its hashed token — logout. Deleting a token that
/// doesn't exist (already logged out, already expired) is not an error;
/// the end state either way is "this token no longer authenticates".
pub async fn delete_session_by_token_hash(pool: &PgPool, token_hash: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-db");
    }

    #[test]
    fn connection_url_percent_encodes_credentials() {
        let config = DatabaseConfig {
            host: "localhost".to_string(),
            port: 5432,
            database: "lisekmovie".to_string(),
            username: "lisek movie".to_string(),
            password: "p@ss/word".to_string(),
        };
        let url = config.to_connection_url();
        assert_eq!(
            url,
            "postgres://lisek%20movie:p%40ss%2Fword@localhost:5432/lisekmovie"
        );
    }

    #[tokio::test]
    async fn connecting_to_an_unreachable_host_times_out_or_fails_cleanly() {
        let config = DatabaseConfig {
            host: "127.0.0.1".to_string(),
            port: 1, // nothing listens here
            database: "lisekmovie".to_string(),
            username: "lisekmovie".to_string(),
            password: "wrong".to_string(),
        };
        let result = connect_with_config(&config).await;
        assert!(matches!(
            result,
            Err(DbError::ConnectionFailed(_)) | Err(DbError::ConnectionTimeout)
        ));
    }
}
