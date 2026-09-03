//! Shared server state and the three-way "what can we serve right now" mode.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use lisek_db::PgPool;
use tokio::sync::RwLock;

/// Where the app keeps everything that isn't in Postgres: right now, just
/// `metadata/{posters,backdrops}/{item_id}.jpg` (see
/// `lisek_metadata::image_cache`). Override with `LISEK_CONFIG_DIR`.
///
/// Resolved to an ABSOLUTE path exactly once per process (cached in a
/// `OnceLock`) and logged the moment that happens. This used to default to
/// the bare relative string `"./config"`, re-resolved fresh against the
/// process's current working directory on *every* call — fine only as
/// long as every launch of the server happens to use the identical
/// working directory, which nothing actually guaranteed: `cargo run` from
/// `lisek-server/`, one of the repo's own `.bat` launchers, an IDE run
/// configuration, and a plain double-clicked `.exe` can each legitimately
/// give the OS process a different starting directory for the exact same
/// binary. A scan run under one of those writes
/// `metadata/posters/{id}.jpg` under *its* `./config`; a later image
/// request served by the process started a different way then looks for
/// that same relative path under a completely different, never-created
/// directory tree — surfacing on Windows as "the system cannot find the
/// path specified" (os error 3, distinct from the plain "file not found"
/// os error 2 a merely-missing single file gives) for an image that was,
/// in fact, downloaded and saved correctly, just under a different
/// absolute path. Anchoring to the running executable's own directory
/// instead of the ambient cwd removes that whole failure mode: however
/// the binary was launched, "next to the .exe" is the same answer every
/// time, on Windows and Linux alike.
pub fn config_dir() -> PathBuf {
    static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
    CONFIG_DIR
        .get_or_init(|| {
            let resolved = match std::env::var("LISEK_CONFIG_DIR") {
                Ok(dir) => {
                    let path = PathBuf::from(dir);
                    if path.is_absolute() {
                        path
                    } else {
                        // An explicit override that's still relative: resolve
                        // it against the current directory once, right here at
                        // startup, rather than leaving it to silently drift
                        // with whatever the cwd happens to be on some later
                        // call — the exact bug this function exists to avoid.
                        std::env::current_dir()
                            .map(|cwd| cwd.join(&path))
                            .unwrap_or(path)
                    }
                }
                Err(_) => exe_dir().join("config"),
            };
            tracing::info!(config_dir = %resolved.display(), "resolved config directory");
            resolved
        })
        .clone()
}

/// The directory the running executable itself lives in. Falls back to
/// `"."` only if the OS genuinely can't report the executable's own path
/// (not expected on any platform this targets, but `PathBuf` has no other
/// infallible "here" to offer instead).
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

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
