//! Shared server state and the three-way "what can we serve right now" mode.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use lisek_db::PgPool;
use arc_swap::ArcSwap;
use lisek_auth::secrets::SecretBox;
use tokio::sync::{Notify, broadcast};

use crate::rate_limit::FailureLimiter;

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
    /// Read on every request: an atomic pointer load, no lock. Replaced as
    /// a whole when the First-Run Wizard finishes (`setup::finish`).
    pub mode: ArcSwap<ServerMode>,
    /// Seals provider API keys before they reach the database.
    pub secrets: SecretBox,
    /// Failed login / password-change counters (`rate_limit`).
    pub auth_failures: FailureLimiter,
    /// Live server events fanned out to WebSocket clients. Slow receivers
    /// lag and skip, which is fine: every event is a full snapshot.
    pub events: broadcast::Sender<ServerEvent>,
    /// Wakes the scan worker when a job is queued; the queue itself is the
    /// `scan_jobs` table.
    pub scan_wakeup: Notify,
    /// Wakes the trickplay worker (after a scan, a regeneration request).
    pub trickplay_wakeup: Notify,
    /// An account is known to exist (set once; accounts are never all deleted).
    pub has_accounts: std::sync::atomic::AtomicBool,
    /// What the trickplay worker is doing right now (admin progress view).
    pub trickplay_progress: std::sync::Mutex<crate::trickplay::Progress>,
    /// Download URLs handed out (`download`).
    pub download_tickets: crate::download::DownloadTickets,
    /// Throttled "last seen" writes for the admin user list (Faza 19).
    pub presence: crate::profile::Presence,
}

impl AppState {
    pub fn new(mode: ServerMode, secrets: SecretBox) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            mode: ArcSwap::from_pointee(mode),
            secrets,
            auth_failures: FailureLimiter::default(),
            events,
            scan_wakeup: Notify::new(),
            trickplay_wakeup: Notify::new(),
            has_accounts: std::sync::atomic::AtomicBool::new(false),
            trickplay_progress: std::sync::Mutex::default(),
            download_tickets: crate::download::DownloadTickets::default(),
            presence: crate::profile::Presence::default(),
        }
    }

    pub fn set_mode(&self, mode: ServerMode) {
        self.mode.store(Arc::new(mode));
    }
}

/// Server-pushed events. Serialized as `{"type": "...", ...}` on the
/// notifications WebSocket.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Admin-only: a scan job changed phase or made progress.
    ScanProgress { job: lisek_db::scan_jobs::ScanJob },
}

impl AppState {
    /// The connection pool, once the server is `Ready`. Used by every
    /// auth-related handler in `auth.rs` and `setup.rs` instead of matching
    /// on `mode` directly.
    pub fn pool(&self) -> Option<PgPool> {
        match &**self.mode.load() {
            ServerMode::Ready(pool) => Some(pool.clone()),
            ServerMode::SetupRequired | ServerMode::DatabaseError(_) => None,
        }
    }
}

pub type SharedState = Arc<AppState>;
