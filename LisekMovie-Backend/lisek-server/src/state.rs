//! Shared server state and the three-way "what can we serve right now" mode.

use std::sync::Arc;

use lisek_db::PgPool;
use tokio::sync::RwLock;

/// What the server can currently do, decided once at startup and updated
/// in place the moment the First-Run Wizard finishes — no restart needed.
pub enum ServerMode {
    /// No saved database connection was found: the wizard hasn't run yet.
    SetupRequired,
    /// A database connection was saved, but it didn't work at startup
    /// (wrong password, container not running, ...). Kept distinct from
    /// `SetupRequired` so the frontend can show "we can't reach your
    /// database" instead of re-running the wizard on an admin whose setup
    /// is actually fine.
    DatabaseError(String),
    /// Fully configured and connected.
    Ready(PgPool),
}

pub struct AppState {
    pub mode: RwLock<ServerMode>,
}

impl AppState {
    /// The connection pool, once the server is `Ready`. Used by every
    /// auth-related handler in `auth.rs` and `setup.rs` instead of matching
    /// on `mode` directly.
    pub async fn pool(&self) -> Option<PgPool> {
        match &*self.mode.read().await {
            ServerMode::Ready(pool) => Some(pool.clone()),
            ServerMode::SetupRequired | ServerMode::DatabaseError(_) => None,
        }
    }
}

pub type SharedState = Arc<AppState>;
