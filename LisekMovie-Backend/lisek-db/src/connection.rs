//! Connecting to PostgreSQL and running the embedded migrations.

use std::time::Duration;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::{DbError, PgPool};

/// How long we're willing to wait for a database to respond before giving
/// up — both during the wizard's "test connection" step and at normal
/// startup. Long enough for a slow local Postgres to wake up, short enough
/// that a wrong host doesn't leave the admin staring at a spinner forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Discrete connection fields, as collected by the First-Run Wizard form.
/// Kept separate from a raw connection-string type so the wizard never has
/// to build (or the frontend parse) a `postgres://` URL by hand.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
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
    let migrator = sqlx::migrate!("./migrations");
    let healed = heal_line_ending_checksums(pool, &migrator).await?;
    if healed > 0 {
        tracing::info!(
            migrations = healed,
            "updated checksums of applied migrations whose files differ only in line endings"
        );
    }
    migrator
        .run(pool)
        .await
        .map_err(|err| DbError::MigrationFailed(err.to_string()))
}

/// sqlx checksums a migration file byte for byte, so the same migration
/// checked out with CRLF (Windows `core.autocrlf`) and LF hashes
/// differently, and the server would refuse to start ("migration was
/// previously applied but has been modified"). For every applied migration
/// whose stored checksum matches the current file with the other line
/// ending, the stored checksum is updated. Any real content change still
/// fails as before.
async fn heal_line_ending_checksums(
    pool: &PgPool,
    migrator: &sqlx::migrate::Migrator,
) -> Result<u64, DbError> {
    let table_exists: bool =
        sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
            .fetch_one(pool)
            .await?;
    if !table_exists {
        return Ok(0);
    }
    let applied: Vec<(i64, Vec<u8>)> =
        sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations")
            .fetch_all(pool)
            .await?;

    let mut healed = 0;
    for migration in migrator.iter() {
        let Some((_, stored)) = applied.iter().find(|(v, _)| *v == migration.version) else {
            continue;
        };
        if stored.as_slice() == &*migration.checksum {
            continue;
        }
        let lf = migration.sql.as_str().replace("\r\n", "\n");
        let crlf = lf.replace('\n', "\r\n");
        if [lf, crlf].into_iter().any(|sql| sql_checksum(migration, sql) == *stored) {
            sqlx::query("UPDATE _sqlx_migrations SET checksum = $1 WHERE version = $2")
                .bind(&*migration.checksum)
                .bind(migration.version)
                .execute(pool)
                .await?;
            healed += 1;
        }
    }
    Ok(healed)
}

/// The checksum sqlx would store for `migration` if its file contained `sql`.
fn sql_checksum(migration: &sqlx::migrate::Migration, sql: String) -> Vec<u8> {
    use sqlx::SqlSafeStr;
    sqlx::migrate::Migration::new(
        migration.version,
        migration.description.clone(),
        migration.migration_type,
        sqlx::AssertSqlSafe(sql).into_sql_str(),
        migration.no_tx,
    )
    .checksum
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

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
