//! PostgreSQL access layer (sqlx) and migrations.
//!
//! One module per domain. Everything the server used before the split is
//! re-exported at the crate root, so call sites keep writing
//! `lisek_db::list_libraries(..)`; newer areas (`scan_jobs`, `streams`,
//! `playback`, ...) are addressed through their module.
//!
//! Queries come in two forms:
//! - `sqlx::query!`/`query_as!` macros (see `catalog`): checked against
//!   the schema at compile time. The workspace builds with
//!   `SQLX_OFFLINE=true` (`.cargo/config.toml`) from the committed
//!   `.sqlx/` cache; after changing such a query, run
//!   `cargo sqlx prepare --workspace` against a migrated database.
//! - runtime `sqlx::query`: used where the SQL text is assembled from
//!   fixed fragments (library paging), and in modules not migrated yet.

mod accounts;
mod catalog;
mod connection;
mod libraries;
mod media;
mod providers;
mod scan;

pub mod access;
pub mod collections;
pub mod episodes;
pub mod games;
pub mod notifications;
pub mod people;
pub mod playback;
pub mod profiles;
pub mod scan_jobs;
pub mod search;
pub mod seasons;
pub mod stats;
pub mod streams;
pub mod trickplay;
pub mod users;

pub use accounts::*;
pub use catalog::*;
pub use connection::*;
pub use libraries::*;
pub use media::*;
pub use providers::*;
pub use scan::*;

pub(crate) use accounts::{UserRow, user_from_row};

pub use sqlx::PgPool;
use thiserror::Error;

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
    #[error("no such provider is registered")]
    ProviderNotFound,
    #[error("'{0}' is not a valid library type")]
    InvalidLibraryType(String),
    #[error("no such library")]
    LibraryNotFound,
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

#[cfg(test)]
mod tests {
    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-db");
    }
}
