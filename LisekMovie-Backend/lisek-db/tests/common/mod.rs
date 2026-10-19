//! Shared harness for the database-backed tests.
//!
//! Opt-in: set `LISEK_TEST_DATABASE_URL` to a server/role allowed to
//! create databases (e.g. `postgres://lisekmovie:lisekmovie@localhost/postgres`).
//! Each test creates and drops its own database. Without the variable the
//! tests print a notice and pass, so `cargo test` never needs Postgres.

use lisek_db::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, ConnectOptions};
use uuid::Uuid;

pub struct TestDb {
    pub pool: PgPool,
    admin: PgPool,
    name: String,
}

impl TestDb {
    pub async fn create() -> Option<TestDb> {
        let Ok(url) = std::env::var("LISEK_TEST_DATABASE_URL") else {
            eprintln!("LISEK_TEST_DATABASE_URL not set; skipping database test");
            return None;
        };
        let admin_options: PgConnectOptions = url.parse().expect("valid LISEK_TEST_DATABASE_URL");
        let admin = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin_options.clone())
            .await
            .unwrap();
        let name = format!("lisek_test_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .unwrap();
        let options = admin_options.database(&name).disable_statement_logging();
        let pool = PgPoolOptions::new().connect_with(options).await.unwrap();
        lisek_db::run_migrations(&pool).await.unwrap();
        Some(TestDb { pool, admin, name })
    }

    /// Reconnects to the same database (a fresh pool), as a restarted
    /// server would.
    #[allow(dead_code)] // used by some test binaries only
    pub async fn reconnect(&self) -> PgPool {
        let url = std::env::var("LISEK_TEST_DATABASE_URL").unwrap();
        let options: PgConnectOptions = url.parse().unwrap();
        PgPoolOptions::new()
            .connect_with(options.database(&self.name).disable_statement_logging())
            .await
            .unwrap()
    }

    pub async fn drop(self) {
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            "DROP DATABASE {} WITH (FORCE)",
            self.name
        )))
        .execute(&self.admin)
        .await
        .unwrap();
    }
}
