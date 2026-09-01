//! PostgreSQL access layer (sqlx) and migrations.
//!
//! Faza 2 added connecting (discrete fields or a saved URL), running
//! migrations, and creating the first admin account. Faza 3 added session
//! storage/lookup for the login flow. Faza 5 adds library CRUD and
//! persisting `lisek-library` scan results into `media_items`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
pub use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use thiserror::Error;
use uuid::Uuid;

use lisek_core::{Library, MediaItem, User};
use lisek_library::{LibraryType, ScanResult};
use lisek_metadata::{ProviderConfig, display_name_for};

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

/// The raw shape every `provider_config` row comes back as. `settings` is
/// kept as a bare `serde_json::Value` here rather than a typed struct —
/// different providers can eventually want different settings shapes
/// (an API key today, maybe a base URL or region override later), and this
/// layer's job is just moving the JSONB in and out, not interpreting it.
type ProviderConfigRow = (String, bool, i32, serde_json::Value);

/// True when `settings` holds a non-empty `api_key` string — the only
/// thing any client of [`list_provider_configs`] gets to know about a
/// provider's settings; the raw value itself never leaves this function.
fn settings_has_api_key(settings: &serde_json::Value) -> bool {
    settings
        .get("api_key")
        .and_then(|value| value.as_str())
        .is_some_and(|key| !key.is_empty())
}

fn provider_config_from_row(row: ProviderConfigRow) -> ProviderConfig {
    let (provider_key, enabled, priority, settings) = row;
    ProviderConfig {
        display_name: display_name_for(&provider_key),
        is_configured: settings_has_api_key(&settings),
        provider_key,
        enabled,
        priority,
    }
}

/// Every registered metadata provider (built-in or, later, an external
/// plugin's), ordered by admin-configured priority. Backs the Admin
/// Panel's plugins tab.
pub async fn list_provider_configs(pool: &PgPool) -> Result<Vec<ProviderConfig>, DbError> {
    let rows = sqlx::query_as::<_, ProviderConfigRow>(
        r#"
        SELECT provider_key, enabled, priority, settings
        FROM provider_config
        ORDER BY priority ASC, provider_key ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(provider_config_from_row).collect())
}

/// Every *enabled and configured* provider's raw API key, in admin-set
/// priority order — server-internal only. Unlike [`list_provider_configs`]
/// (which deliberately never exposes a stored key, only whether one
/// exists — see [`ProviderConfig`]'s own doc comment), this is what lets
/// `lisek-server` actually build a `TmdbClient`/`TvdbClient` to call out to
/// a provider on the admin's behalf; nothing here is ever serialized back
/// to an HTTP response.
pub async fn list_enabled_provider_keys(pool: &PgPool) -> Result<Vec<(String, String)>, DbError> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT provider_key, settings
        FROM provider_config
        WHERE enabled
        ORDER BY priority ASC, provider_key ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(provider_key, settings)| {
            let api_key = settings.get("api_key")?.as_str()?;
            if api_key.is_empty() {
                return None;
            }
            Some((provider_key, api_key.to_string()))
        })
        .collect())
}

/// Updates a provider's enabled state and display priority. Returns
/// [`DbError::ProviderNotFound`] if `provider_key` doesn't match a row —
/// callers use this to tell "updated" apart from "no such provider" without
/// a separate existence check.
pub async fn set_provider_enabled_and_priority(
    pool: &PgPool,
    provider_key: &str,
    enabled: bool,
    priority: i32,
) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET enabled = $2, priority = $3
        WHERE provider_key = $1
        "#,
    )
    .bind(provider_key)
    .bind(enabled)
    .bind(priority)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::ProviderNotFound);
    }
    Ok(())
}

/// Sets (or replaces) a provider's API key. Merges into the existing
/// `settings` JSONB via `||` rather than overwriting it outright, so a
/// future second settings field (a base URL, a region) set independently
/// isn't clobbered by this call.
pub async fn set_provider_api_key(
    pool: &PgPool,
    provider_key: &str,
    api_key: &str,
) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET settings = settings || jsonb_build_object('api_key', $2::text)
        WHERE provider_key = $1
        "#,
    )
    .bind(provider_key)
    .bind(api_key)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::ProviderNotFound);
    }
    Ok(())
}

/// Removes a provider's API key (if any) without touching any other
/// settings field, or its enabled/priority state.
pub async fn clear_provider_api_key(pool: &PgPool, provider_key: &str) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET settings = settings - 'api_key'
        WHERE provider_key = $1
        "#,
    )
    .bind(provider_key)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::ProviderNotFound);
    }
    Ok(())
}

/// The row shape a `libraries` SELECT comes back as.
type LibraryRow = (Uuid, String, String, i32, Vec<String>, bool, DateTime<Utc>);

fn library_from_row(row: LibraryRow) -> Library {
    Library {
        id: row.0,
        name: row.1,
        library_type: row.2,
        sort_order: row.3,
        root_paths: row.4,
        save_metadata_in_media_folder: row.5,
        created_at: row.6,
    }
}

/// Creates a new library, appended after every existing one in sort order.
/// `library_type` must be one of [`lisek_library::LibraryType`]'s canonical
/// DB strings (`"movies"` / `"tv_shows"`) — anything else comes back as
/// [`DbError::InvalidLibraryType`] before any query runs, so a typo from a
/// future caller can't silently create a library nothing can ever scan.
pub async fn create_library(
    pool: &PgPool,
    name: &str,
    library_type: &str,
    root_paths: &[String],
    save_metadata_in_media_folder: bool,
) -> Result<Library, DbError> {
    if LibraryType::from_db_str(library_type).is_none() {
        return Err(DbError::InvalidLibraryType(library_type.to_string()));
    }

    let next_sort_order: i32 =
        sqlx::query_scalar("SELECT COALESCE(MAX(sort_order) + 1, 0) FROM libraries")
            .fetch_one(pool)
            .await?;

    let row = sqlx::query_as::<_, LibraryRow>(
        r#"
        INSERT INTO libraries (name, library_type, sort_order, root_paths, save_metadata_in_media_folder)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id, name, library_type, sort_order, root_paths, save_metadata_in_media_folder, created_at
        "#,
    )
    .bind(name)
    .bind(library_type)
    .bind(next_sort_order)
    .bind(root_paths)
    .bind(save_metadata_in_media_folder)
    .fetch_one(pool)
    .await?;

    Ok(library_from_row(row))
}

/// Every configured library, in the admin's chosen display order.
pub async fn list_libraries(pool: &PgPool) -> Result<Vec<Library>, DbError> {
    let rows = sqlx::query_as::<_, LibraryRow>(
        r#"
        SELECT id, name, library_type, sort_order, root_paths, save_metadata_in_media_folder, created_at
        FROM libraries
        ORDER BY sort_order ASC, name ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(library_from_row).collect())
}

/// Looks up one library by id — used by the scan-trigger endpoint to read
/// its `root_paths` and `library_type` before walking the filesystem, and
/// its `save_metadata_in_media_folder` before enriching any item it finds.
pub async fn get_library(pool: &PgPool, library_id: Uuid) -> Result<Option<Library>, DbError> {
    let row = sqlx::query_as::<_, LibraryRow>(
        r#"
        SELECT id, name, library_type, sort_order, root_paths, save_metadata_in_media_folder, created_at
        FROM libraries
        WHERE id = $1
        "#,
    )
    .bind(library_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(library_from_row))
}

/// Deletes a library and, via `ON DELETE CASCADE`, every media item and
/// per-library provider override that belonged to it.
pub async fn delete_library(pool: &PgPool, library_id: Uuid) -> Result<(), DbError> {
    let result = sqlx::query("DELETE FROM libraries WHERE id = $1")
        .bind(library_id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::LibraryNotFound);
    }
    Ok(())
}

/// How many of each outcome a scan produced — enough for the admin-facing
/// "scan complete" summary without reporting on every single file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ScanSummary {
    pub movies_found: usize,
    pub episodes_found: usize,
    pub unrecognized: usize,
}

/// What a freshly-upserted `media_items` row still needs looked up from a
/// metadata provider: a movie's title/year, or a series' title.
/// `lisek-db` stays free of any HTTP/provider-API knowledge (see this
/// crate's own doc comment on its boundaries) — [`persist_scan_results`]
/// hands these back to its caller (`lisek-server`, where API keys and the
/// `lisek-metadata` TMDB/TVDB clients live) rather than trying to fetch
/// metadata itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrichTarget {
    Movie { title: String, year: Option<i32> },
    Series { title: String },
}

/// The result of persisting one scan: the admin-facing summary, plus the
/// id, lookup target, and on-disk file path of every item worth a
/// metadata-provider request — which excludes both `Unrecognized` files
/// (nothing to persist for those at all) and any existing row left
/// untouched because `metadata_locked` is set (an admin's manual
/// correction is never overwritten, and is never worth spending a
/// provider request re-confirming either). The file path travels
/// alongside the id/target so the caller can write a provider match's
/// poster/backdrop next to it (see `lisek_metadata::image_cache::
/// save_image`) without a second round trip to look it back up.
pub struct PersistedScan {
    pub summary: ScanSummary,
    pub enrich_targets: Vec<(Uuid, EnrichTarget, PathBuf)>,
}

/// Upserts one item, returning its id only when the row was actually
/// inserted or updated. `None` means an existing row was found but left
/// untouched because it's `metadata_locked` — Postgres's `ON CONFLICT ...
/// DO UPDATE ... WHERE` skips (and `RETURNING` reports nothing for) a
/// conflicting row whose `WHERE` condition evaluates false, which is
/// exactly the "was this actually applied" signal the caller needs to
/// decide whether the item is worth a metadata-provider lookup at all.
async fn upsert_media_item(
    pool: &PgPool,
    library_id: Uuid,
    item_type: &str,
    title: &str,
    file_path: &Path,
    metadata: &serde_json::Value,
) -> Result<Option<Uuid>, DbError> {
    let file_path = file_path.to_string_lossy();
    let id: Option<(Uuid,)> = sqlx::query_as(
        r#"
        INSERT INTO media_items (library_id, item_type, title, file_path, metadata)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (library_id, file_path)
        DO UPDATE SET title = EXCLUDED.title, metadata = EXCLUDED.metadata, updated_at = now()
        WHERE NOT media_items.metadata_locked
        RETURNING id
        "#,
    )
    .bind(library_id)
    .bind(item_type)
    .bind(title)
    .bind(file_path.as_ref())
    .bind(metadata)
    .fetch_optional(pool)
    .await?;

    Ok(id.map(|(id,)| id))
}

/// Persists every [`ScanResult`] from a library scan into `media_items`,
/// upserting on `(library_id, file_path)` (see the Faza 5 migration that
/// added that uniqueness constraint) so re-scanning a library updates
/// existing rows instead of duplicating them. `Unrecognized` files are
/// counted in the summary but never written — there's nothing to persist
/// for a file the parser couldn't identify.
pub async fn persist_scan_results(
    pool: &PgPool,
    library_id: Uuid,
    results: &[ScanResult],
) -> Result<PersistedScan, DbError> {
    let mut summary = ScanSummary::default();
    let mut enrich_targets = Vec::new();

    for result in results {
        match result {
            ScanResult::Movie(movie) => {
                let metadata = serde_json::json!({ "year": movie.year });
                let id = upsert_media_item(
                    pool,
                    library_id,
                    "movie",
                    &movie.title,
                    &movie.file_path,
                    &metadata,
                )
                .await?;
                if let Some(id) = id {
                    enrich_targets.push((
                        id,
                        EnrichTarget::Movie {
                            title: movie.title.clone(),
                            year: movie.year,
                        },
                        movie.file_path.clone(),
                    ));
                }
                summary.movies_found += 1;
            }
            ScanResult::Episode(episode) => {
                let metadata = serde_json::json!({
                    "season": episode.season,
                    "episode": episode.episode,
                    "end_episode": episode.end_episode,
                });
                let id = upsert_media_item(
                    pool,
                    library_id,
                    "episode",
                    &episode.series_title,
                    &episode.file_path,
                    &metadata,
                )
                .await?;
                if let Some(id) = id {
                    enrich_targets.push((
                        id,
                        EnrichTarget::Series {
                            title: episode.series_title.clone(),
                        },
                        episode.file_path.clone(),
                    ));
                }
                summary.episodes_found += 1;
            }
            ScanResult::Unrecognized(_) => summary.unrecognized += 1,
        }
    }

    Ok(PersistedScan {
        summary,
        enrich_targets,
    })
}

/// Merges a metadata-provider match into an item's `metadata` JSONB. No
/// `metadata_locked` check here — the caller only ever reaches this for an
/// id [`persist_scan_results`] already confirmed was just inserted or
/// unlocked-and-updated, so a lock set concurrently between that call and
/// this one is the only way this could race, an acceptable gap for a
/// same-request follow-up write. Uses `||` (merge), not overwrite, so
/// provider fields sit alongside the scan-derived ones (`year`,
/// `season`/`episode`) already written by [`upsert_media_item`] rather than
/// replacing them.
pub async fn attach_provider_metadata(
    pool: &PgPool,
    item_id: Uuid,
    provider_key: &str,
    fields: &serde_json::Value,
) -> Result<(), DbError> {
    let metadata = serde_json::json!({ "provider": provider_key, "provider_data": fields });
    sqlx::query(
        r#"
        UPDATE media_items
        SET metadata = metadata || $2::jsonb, updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(item_id)
    .bind(metadata)
    .execute(pool)
    .await?;

    Ok(())
}

/// Records whether an item's provider images ended up in the local
/// config-dir cache only because a media-folder write was attempted and
/// denied (see `lisek_metadata::image_cache::save_image`), rather than
/// because the library simply has media-folder saving turned off. Called
/// once per enriched item, every scan, so a permission problem that gets
/// fixed later and a subsequent successful media-folder write correctly
/// clears the flag again rather than leaving a stale warning behind.
pub async fn set_metadata_fallback(
    pool: &PgPool,
    item_id: Uuid,
    fallback: bool,
) -> Result<(), DbError> {
    sqlx::query("UPDATE media_items SET metadata_fallback = $2 WHERE id = $1")
        .bind(item_id)
        .bind(fallback)
        .execute(pool)
        .await?;

    Ok(())
}

/// One item's on-disk file path — used by the image-serving routes to
/// check for a poster/backdrop written next to the media itself before
/// falling back to the local config-dir cache (see `lisek-server`'s
/// `media.rs`). `None` when no such item exists at all, which the caller
/// treats the same as "no media-folder candidate, try the cache."
pub async fn get_media_item_file_path(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<String>, DbError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT file_path FROM media_items WHERE id = $1")
        .bind(item_id)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|(file_path,)| file_path))
}

/// The raw shape every `media_items` row comes back as for the read side
/// (the home feed, and later a full browse view) — `year`/`overview`/
/// `season`/`episode` are pulled out of the `metadata` JSONB blob right in
/// SQL rather than deserializing it in Rust, since every reader so far only
/// ever wants these four scalars out of it, never the whole document.
/// `overview` specifically comes from `metadata->'provider_data'->>
/// 'overview'` — see `attach_provider_metadata`'s merge shape above — so it
/// stays `NULL` until a provider has actually matched the item.
type MediaItemRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<i32>,
    Option<String>,
    Option<i32>,
    Option<i32>,
    DateTime<Utc>,
);

fn media_item_from_row(row: MediaItemRow) -> MediaItem {
    MediaItem {
        id: row.0,
        library_id: row.1,
        item_type: row.2,
        title: row.3,
        year: row.4,
        overview: row.5,
        season: row.6,
        episode: row.7,
        created_at: row.8,
    }
}

/// Every read query below selects the exact same nine columns in the exact
/// same order as [`MediaItemRow`] (a reordered `SELECT` would silently
/// scramble which tuple field gets which value, since `sqlx::query_as` on a
/// tuple binds positionally, not by column name) — spelled out in full in
/// each query rather than built from one shared constant, since sqlx 0.9's
/// injection-safety check only accepts a genuine `&'static str` literal per
/// call site, not one assembled at runtime via `format!`.
///
/// The single item the Hero Banner features: preferring one a metadata
/// provider has actually matched (so there's a real synopsis to show)
/// over just whatever was scanned most recently, but falling back to plain
/// scan recency once nothing is enriched yet (e.g. right after a library's
/// very first scan, before any provider is configured) rather than leaving
/// the banner empty. Postgres orders `false < true`, so `DESC` on the
/// "has an overview" test puts an enriched row first without a `CASE`.
pub async fn get_hero_item(pool: &PgPool) -> Result<Option<MediaItem>, DbError> {
    let row = sqlx::query_as::<_, MediaItemRow>(
        r#"
        SELECT id, library_id, item_type, title,
               (metadata->>'year')::int AS year,
               metadata->'provider_data'->>'overview' AS overview,
               (metadata->>'season')::int AS season,
               (metadata->>'episode')::int AS episode,
               created_at
        FROM media_items
        ORDER BY (metadata->'provider_data'->>'overview' IS NOT NULL) DESC, created_at DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(media_item_from_row))
}

/// The most recently scanned-or-rescanned items, newest first — the one row
/// with a genuinely unambiguous, non-placeholder ordering.
pub async fn list_recently_added(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        SELECT id, library_id, item_type, title,
               (metadata->>'year')::int AS year,
               metadata->'provider_data'->>'overview' AS overview,
               (metadata->>'season')::int AS season,
               (metadata->>'episode')::int AS episode,
               created_at
        FROM media_items
        ORDER BY created_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(media_item_from_row).collect())
}

/// "Top 10 Today"'s backing query. There's no watch-count or trending
/// signal anywhere in this schema yet (that's real usage-analytics work,
/// planned as its own later phase — see the roadmap) — rather than fake a
/// ranking or just repeat `list_recently_added` under a second label, this
/// orders by release year (then scan recency as a tiebreaker), which reads
/// honestly as "the newest releases in your library" instead of claiming
/// to be a trending chart it has no data to back up.
pub async fn list_top_today(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        SELECT id, library_id, item_type, title,
               (metadata->>'year')::int AS year,
               metadata->'provider_data'->>'overview' AS overview,
               (metadata->>'season')::int AS season,
               (metadata->>'episode')::int AS episode,
               created_at
        FROM media_items
        ORDER BY (metadata->>'year')::int DESC NULLS LAST, created_at DESC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(media_item_from_row).collect())
}

/// "Recommended For You"'s backing query. No per-account taste model exists
/// yet (no watch history to learn from at all) — a genuine random sample of
/// the library is the honest placeholder until real personalization is
/// built, rather than dressing up `list_recently_added` again under a
/// different heading. Each call reshuffles independently since Postgres
/// evaluates `random()` fresh per query.
pub async fn list_recommended(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        SELECT id, library_id, item_type, title,
               (metadata->>'year')::int AS year,
               metadata->'provider_data'->>'overview' AS overview,
               (metadata->>'season')::int AS season,
               (metadata->>'episode')::int AS episode,
               created_at
        FROM media_items
        ORDER BY random()
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(media_item_from_row).collect())
}

/// "Popular Now"'s backing query — same caveat as [`list_recommended`]:
/// no view/watch-count tracking exists yet either, so this is another
/// honest random sample rather than a fabricated popularity rank.
pub async fn list_popular(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        SELECT id, library_id, item_type, title,
               (metadata->>'year')::int AS year,
               metadata->'provider_data'->>'overview' AS overview,
               (metadata->>'season')::int AS season,
               (metadata->>'episode')::int AS episode,
               created_at
        FROM media_items
        ORDER BY random()
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(media_item_from_row).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-db");
    }

    #[test]
    fn settings_has_api_key_is_false_for_empty_or_missing() {
        assert!(!settings_has_api_key(&serde_json::json!({})));
        assert!(!settings_has_api_key(&serde_json::json!({ "api_key": "" })));
        assert!(!settings_has_api_key(&serde_json::json!({ "other": "x" })));
    }

    #[test]
    fn settings_has_api_key_is_true_for_a_non_empty_key() {
        assert!(settings_has_api_key(
            &serde_json::json!({ "api_key": "abc123" })
        ));
    }

    #[test]
    fn provider_config_from_row_resolves_display_name_and_configured_state() {
        let config = provider_config_from_row((
            "tmdb".to_string(),
            true,
            0,
            serde_json::json!({ "api_key": "secret" }),
        ));
        assert_eq!(config.provider_key, "tmdb");
        assert_eq!(config.display_name, "TMDB");
        assert!(config.enabled);
        assert!(config.is_configured);
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
