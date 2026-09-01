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
    /// `(item_id, target, file_path, forced_provider)` — `forced_provider`
    /// is this item's own admin-set override (see the metadata editor's
    /// "Pola aktywne" section), carried straight through from the upsert
    /// below so the caller can skip straight to that one provider instead
    /// of the admin's normal priority list. `None` for the overwhelming
    /// majority of items that have never had one set.
    pub enrich_targets: Vec<(Uuid, EnrichTarget, PathBuf, Option<String>)>,
}

/// Upserts one item, returning its id (and current `forced_provider`) only
/// when the row was actually inserted or updated. `None` means an existing
/// row was found but left untouched because it's `metadata_locked` —
/// Postgres's `ON CONFLICT ... DO UPDATE ... WHERE` skips (and `RETURNING`
/// reports nothing for) a conflicting row whose `WHERE` condition
/// evaluates false, which is exactly the "was this actually applied"
/// signal the caller needs to decide whether the item is worth a
/// metadata-provider lookup at all.
///
/// A fully `metadata_locked` item skips this whole upsert (as before); a
/// merely partially-protected one (an individual field in `locked_fields`,
/// set from the same editor section) still gets scanned and re-enriched
/// normally, except that field specifically keeps its old value:
/// `title` when `'title'` is locked, `year` when `'year'` is locked — the
/// `CASE` expressions below reference the pre-update row (plain
/// `media_items.*`, not `EXCLUDED.*`) to pull the value back in.
/// `original_title` (never scanner-derived — see the Faza 5-completion
/// migration) and `provider_data` (owned by [`attach_provider_metadata`],
/// applied in a separate follow-up call) are carried forward unconditionally
/// either way, since this function has no fresh value for either of them
/// and would otherwise silently erase them on every re-scan.
async fn upsert_media_item(
    pool: &PgPool,
    library_id: Uuid,
    item_type: &str,
    title: &str,
    file_path: &Path,
    metadata: &serde_json::Value,
) -> Result<Option<(Uuid, Option<String>)>, DbError> {
    let file_path = file_path.to_string_lossy();
    let row: Option<(Uuid, Option<String>)> = sqlx::query_as(
        r#"
        INSERT INTO media_items (library_id, item_type, title, file_path, metadata)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (library_id, file_path)
        DO UPDATE SET
            title = CASE
                WHEN 'title' = ANY(media_items.locked_fields) THEN media_items.title
                ELSE EXCLUDED.title
            END,
            metadata = EXCLUDED.metadata
                || CASE WHEN media_items.metadata ? 'original_title'
                        THEN jsonb_build_object('original_title', media_items.metadata->'original_title')
                        ELSE '{}'::jsonb END
                || CASE WHEN media_items.metadata ? 'provider_data'
                        THEN jsonb_build_object('provider_data', media_items.metadata->'provider_data')
                        ELSE '{}'::jsonb END
                || CASE WHEN 'year' = ANY(media_items.locked_fields) AND media_items.metadata ? 'year'
                        THEN jsonb_build_object('year', media_items.metadata->'year')
                        ELSE '{}'::jsonb END,
            updated_at = now()
        WHERE NOT media_items.metadata_locked
        RETURNING id, forced_provider
        "#,
    )
    .bind(library_id)
    .bind(item_type)
    .bind(title)
    .bind(file_path.as_ref())
    .bind(metadata)
    .fetch_optional(pool)
    .await?;

    Ok(row)
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
                let row = upsert_media_item(
                    pool,
                    library_id,
                    "movie",
                    &movie.title,
                    &movie.file_path,
                    &metadata,
                )
                .await?;
                if let Some((id, forced_provider)) = row {
                    enrich_targets.push((
                        id,
                        EnrichTarget::Movie {
                            title: movie.title.clone(),
                            year: movie.year,
                        },
                        movie.file_path.clone(),
                        forced_provider,
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
                let row = upsert_media_item(
                    pool,
                    library_id,
                    "episode",
                    &episode.series_title,
                    &episode.file_path,
                    &metadata,
                )
                .await?;
                if let Some((id, forced_provider)) = row {
                    enrich_targets.push((
                        id,
                        EnrichTarget::Series {
                            title: episode.series_title.clone(),
                        },
                        episode.file_path.clone(),
                        forced_provider,
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
///
/// Respects the item's own `locked_fields` — here that's `overview`
/// specifically: when locked, the freshly-fetched `fields.overview` this
/// call was about to write is swapped back out for whatever the item's
/// `provider_data` already had, right inside the query (`locked_fields`
/// and the old `provider_data` are both read live off the row being
/// updated, not passed in from the caller — so this stays correct however
/// the item got locked, whether that was before or after the caller
/// fetched `fields`), so a provider re-match never quietly overwrites an
/// admin's manual description edit. Every other key in `fields`
/// (`tmdb_id`/`tvdb_id`) still updates normally either way — only the
/// single locked key reverts.
pub async fn attach_provider_metadata(
    pool: &PgPool,
    item_id: Uuid,
    provider_key: &str,
    fields: &serde_json::Value,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        UPDATE media_items
        SET metadata = metadata || jsonb_build_object(
                'provider', $2::text,
                'provider_data',
                ($3::jsonb) || CASE
                    WHEN 'overview' = ANY(locked_fields) AND metadata->'provider_data' ? 'overview'
                    THEN jsonb_build_object('overview', metadata->'provider_data'->'overview')
                    ELSE '{}'::jsonb
                END
            ),
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(item_id)
    .bind(provider_key)
    .bind(fields)
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

/// One item's full editable shape — what the metadata editor loads and
/// what it hands back for [`update_media_item`]. `original_title` reads
/// straight from `metadata->>'original_title'` (see the Faza
/// 5-completion migration's doc comment for why that's a JSONB key, not a
/// column) and `overview` from the same `provider_data` slot the home feed
/// already reads (a manual edit and a provider match both land there —
/// see [`update_media_item`]). `locked_fields` names, verbatim, whichever
/// of `"title"`/`"original_title"`/`"year"`/`"overview"` the admin has
/// individually protected — the same identifiers the editor's own
/// checkboxes use, so the frontend never needs a translation table.
#[derive(Debug, Clone, Serialize)]
pub struct MediaItemDetail {
    pub id: Uuid,
    pub library_id: Uuid,
    pub item_type: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub metadata_locked: bool,
    pub forced_provider: Option<String>,
    pub locked_fields: Vec<String>,
}

type MediaItemDetailRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<String>,
    Option<i32>,
    Option<String>,
    Option<i32>,
    Option<i32>,
    bool,
    Option<String>,
    Vec<String>,
);

fn media_item_detail_from_row(row: MediaItemDetailRow) -> MediaItemDetail {
    MediaItemDetail {
        id: row.0,
        library_id: row.1,
        item_type: row.2,
        title: row.3,
        original_title: row.4,
        year: row.5,
        overview: row.6,
        season: row.7,
        episode: row.8,
        metadata_locked: row.9,
        forced_provider: row.10,
        locked_fields: row.11,
    }
}

/// Loads one item for the metadata editor. `None` when no such item
/// exists (a stale id from a deleted/re-scanned library) — the caller
/// turns that into a 404, same as every other single-resource lookup in
/// this crate.
pub async fn get_media_item_detail(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<MediaItemDetail>, DbError> {
    let row = sqlx::query_as::<_, MediaItemDetailRow>(
        r#"
        SELECT id, library_id, item_type, title,
               metadata->>'original_title' AS original_title,
               (metadata->>'year')::int AS year,
               metadata->'provider_data'->>'overview' AS overview,
               (metadata->>'season')::int AS season,
               (metadata->>'episode')::int AS episode,
               metadata_locked, forced_provider, locked_fields
        FROM media_items
        WHERE id = $1
        "#,
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(media_item_detail_from_row))
}

/// The metadata editor's save action — an admin's own edit, so unlike
/// every automatic write in this module, this one is never gated on
/// `locked_fields` (those only ever protect a field from a *future
/// automatic* re-scan/refresh, never from the admin editing it directly
/// right now) and always applies every field, including setting
/// `metadata_locked`/`forced_provider`/`locked_fields` themselves — this
/// is the one place all three actually get written. `overview` lands in
/// the same `metadata->'provider_data'->>'overview'` slot a provider match
/// already uses (see [`attach_provider_metadata`]), so every reader keeps
/// working unchanged regardless of whether the current value came from an
/// admin or a provider; `tmdb_id`/`tvdb_id`/`provider`, if already present
/// in `provider_data`, survive this call untouched. Returns `false` (no
/// error) for an id that doesn't exist, same "not found, not a failure"
/// shape as [`delete_library`].
#[allow(clippy::too_many_arguments)]
pub async fn update_media_item(
    pool: &PgPool,
    item_id: Uuid,
    title: &str,
    original_title: Option<&str>,
    year: Option<i32>,
    overview: Option<&str>,
    metadata_locked: bool,
    forced_provider: Option<&str>,
    locked_fields: &[String],
) -> Result<bool, DbError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r#"
        UPDATE media_items
        SET title = $2,
            metadata = (metadata - 'original_title' - 'year' - 'provider_data')
                || jsonb_build_object('original_title', $3::text)
                || jsonb_build_object('year', $4::int)
                || jsonb_build_object(
                    'provider_data',
                    COALESCE(metadata->'provider_data', '{}'::jsonb)
                        || jsonb_build_object('overview', $5::text)
                ),
            metadata_locked = $6,
            forced_provider = $7,
            locked_fields = $8,
            updated_at = now()
        WHERE id = $1
        RETURNING id
        "#,
    )
    .bind(item_id)
    .bind(title)
    .bind(original_title)
    .bind(year)
    .bind(overview)
    .bind(metadata_locked)
    .bind(forced_provider)
    .bind(locked_fields)
    .fetch_optional(pool)
    .await?;

    Ok(row.is_some())
}

/// Every item in one library, for the Admin Panel's browse view that opens
/// the metadata editor — deliberately not paginated yet (a self-hosted
/// library's realistic size doesn't need it at Faza 5's scope; revisit if
/// that stops being true). `metadata_locked`/`metadata_fallback` ride
/// along so the browse list itself can show a lock/warning badge without a
/// second request per row.
#[derive(Debug, Clone, Serialize)]
pub struct MediaItemSummary {
    pub id: Uuid,
    pub item_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub metadata_locked: bool,
    pub metadata_fallback: bool,
}

type MediaItemSummaryRow = (
    Uuid,
    String,
    String,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    bool,
    bool,
);

pub async fn list_media_items_for_library(
    pool: &PgPool,
    library_id: Uuid,
) -> Result<Vec<MediaItemSummary>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemSummaryRow>(
        r#"
        SELECT id, item_type, title,
               (metadata->>'year')::int AS year,
               (metadata->>'season')::int AS season,
               (metadata->>'episode')::int AS episode,
               metadata_locked, metadata_fallback
        FROM media_items
        WHERE library_id = $1
        ORDER BY title ASC
        "#,
    )
    .bind(library_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, item_type, title, year, season, episode, metadata_locked, metadata_fallback)| {
                MediaItemSummary {
                    id,
                    item_type,
                    title,
                    year,
                    season,
                    episode,
                    metadata_locked,
                    metadata_fallback,
                }
            },
        )
        .collect())
}

/// Every item currently flagged `metadata_fallback` — the Admin Panel's
/// warning view (see the Faza 5-completion roadmap item) lists these with
/// a "move to the media folder" action per row, so the admin doesn't have
/// to `SELECT` the database by hand to find out which items only ever got
/// their poster/backdrop into the local cache. `file_path` rides along so
/// the caller (`lisek-server`) can derive the exact media-folder
/// destination the same way `metadata_enrichment::media_folder_destination`
/// already does, without a second lookup.
#[derive(Debug, Clone, Serialize)]
pub struct FallbackItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub title: String,
    pub file_path: String,
}

pub async fn list_fallback_items(pool: &PgPool) -> Result<Vec<FallbackItem>, DbError> {
    let rows: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
        r#"
        SELECT id, library_id, title, file_path
        FROM media_items
        WHERE metadata_fallback = true
        ORDER BY title ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, library_id, title, file_path)| FallbackItem {
            id,
            library_id,
            title,
            file_path,
        })
        .collect())
}

/// A library's saved custom provider order for one item type (`"movie"` or
/// `"episode"`) — `library_provider_config`'s own Faza 1 schema, unused
/// by any endpoint until the Admin Panel's drag-and-drop reordering UI.
/// `None` means no override has ever been saved for this (library,
/// item_type) pair, which the caller treats as "use the admin's global
/// priority order" (see `lisek-server`'s `provider_registry` doc comment
/// on where that global order itself comes from).
pub async fn get_library_provider_order(
    pool: &PgPool,
    library_id: Uuid,
    item_type: &str,
) -> Result<Option<Vec<String>>, DbError> {
    let row: Option<(Vec<String>,)> = sqlx::query_as(
        "SELECT provider_order FROM library_provider_config WHERE library_id = $1 AND item_type = $2",
    )
    .bind(library_id)
    .bind(item_type)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(order,)| order))
}

/// Saves a library's custom provider order for one item type — an upsert,
/// since the Admin Panel's drag-and-drop list always submits the whole
/// order in one request rather than one move at a time.
pub async fn set_library_provider_order(
    pool: &PgPool,
    library_id: Uuid,
    item_type: &str,
    order: &[String],
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO library_provider_config (library_id, item_type, provider_order)
        VALUES ($1, $2, $3)
        ON CONFLICT (library_id, item_type) DO UPDATE SET provider_order = EXCLUDED.provider_order
        "#,
    )
    .bind(library_id)
    .bind(item_type)
    .bind(order)
    .execute(pool)
    .await?;

    Ok(())
}

/// Every item eligible for the background metadata-refresh job: not
/// `metadata_locked` (the same gate [`upsert_media_item`] already applies
/// to a fresh scan — a locked item is exempt from automatic changes
/// either way), enough to rebuild its [`EnrichTarget`], and its
/// `forced_provider` so the refresh job can respect it exactly the way a
/// scan's own enrichment pass does. Individual `locked_fields` need no
/// equivalent here — [`attach_provider_metadata`] reads that column live
/// off the row it's updating, not from anything this function hands back.
/// `library_id` rides along so the caller can look up each item's library
/// (for `save_metadata_in_media_folder` and any per-library provider
/// order) without a second per-item query.
/// Raw row shape for [`list_unlocked_items_for_refresh`]'s query — factored
/// out purely to keep that function's own signature and query-binding line
/// readable (clippy's `type_complexity` lint).
type UnlockedItemRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<i32>,
    String,
    Option<String>,
);

pub async fn list_unlocked_items_for_refresh(
    pool: &PgPool,
) -> Result<Vec<(Uuid, Uuid, EnrichTarget, PathBuf, Option<String>)>, DbError> {
    let rows: Vec<UnlockedItemRow> = sqlx::query_as(
        r#"
        SELECT id, library_id, item_type, title,
               (metadata->>'year')::int AS year,
               file_path, forced_provider
        FROM media_items
        WHERE NOT metadata_locked
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, library_id, item_type, title, year, file_path, forced_provider)| {
                let target = if item_type == "movie" {
                    EnrichTarget::Movie { title, year }
                } else {
                    EnrichTarget::Series { title }
                };
                (
                    id,
                    library_id,
                    target,
                    PathBuf::from(file_path),
                    forced_provider,
                )
            },
        )
        .collect())
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

// Runda 8: every one of the five home-feed queries below now reads from a
// `home_candidates` CTE instead of `media_items` directly. The bug it
// fixes: a library scan writes one `media_items` row *per episode file*
// (`item_type = 'episode'`), so once a single TV show existed, every query
// here started handing back every one of its episodes as its own card — a
// 20-episode season meant 20 near-identical cards in "Ostatnio dodane"
// alone, reported after the first TV library was scanned.
//
// There is no `series`/`season` row anywhere in this schema (see
// `lisek_core`'s doc comment on `MediaItem::item_type` — the scanner only
// ever writes "movie" or "episode") and this round deliberately does NOT
// add one as a real table: every episode of one show already shares the
// exact same `library_id` + `title` (episodes are upserted under their
// *series* title, never a per-episode one — see `persist_scan_results`),
// and the exact same provider match too, since `EnrichTarget::Series`
// enriches by series title, not per episode. That makes "the show" fully
// derivable as `GROUP BY (library_id, title)` over `item_type = 'episode'`
// rows, with no migration needed. The CTE below does exactly that:
//   - every `movie` row passes through unchanged;
//   - every distinct `(library_id, title)` group of `episode` rows
//     collapses to ONE synthetic row, `item_type = 'series'` (a value that
//     never exists in the table itself — computed here, same idea as
//     `EnrichTarget::Series` on the write side), picking the single most
//     recently-added episode in that group as its representative `id`
//     (`DISTINCT ON` ordered by `created_at DESC, id DESC` — the `id`
//     tie-break makes the pick fully deterministic even for two episodes
//     inserted in the same transaction). `year`/`season`/`episode` are
//     `NULL` for a series row on purpose: none of those describe the show
//     as a whole (a season/episode number is one specific episode's, and
//     — unlike a movie — `ScannedEpisode` never even carries a year at
//     all), and showing "the latest episode's" numbers as if they were the
//     show's own would be inventing a fact this schema doesn't have.
// The representative id is a perfectly real `media_items` row, so
// `GET /api/media/{id}/poster|backdrop|logo` keeps working unmodified —
// every episode of a show was enriched (and had its images cached) on its
// own row, all from the same series-level provider match, so whichever
// episode `DISTINCT ON` happens to pick has its own valid poster/backdrop
// /logo files already sitting wherever `serve_item_image` already looks.
//
// Spelled out in full at every one of the five call sites below (each
// still selects the same nine `MediaItemRow` columns) — sqlx 0.9's
// injection-safety check only accepts a genuine `&'static str` literal per
// call site, not one assembled at runtime via `format!`, and a `const
// &str` fragment can't help either, since the literal each call passes to
// `sqlx::query_as` has to be one complete string token, not several
// concatenated at compile time. This comment is the single source of
// truth for what's otherwise five identical copies of:
//
// ```sql
// WITH home_candidates AS (
//     SELECT id, library_id, item_type, title,
//            (metadata->>'year')::int AS year,
//            metadata->'provider_data'->>'overview' AS overview,
//            NULL::int AS season,
//            NULL::int AS episode,
//            created_at
//     FROM media_items
//     WHERE item_type = 'movie'
//
//     UNION ALL
//
//     SELECT DISTINCT ON (library_id, title)
//            id, library_id, 'series' AS item_type, title,
//            NULL::int AS year,
//            metadata->'provider_data'->>'overview' AS overview,
//            NULL::int AS season,
//            NULL::int AS episode,
//            created_at
//     FROM media_items
//     WHERE item_type = 'episode'
//     ORDER BY library_id, title, created_at DESC, id DESC
// )
// ```

/// The single item the Hero Banner features: preferring one a metadata
/// provider has actually matched (so there's a real synopsis to show)
/// over just whatever was scanned most recently, but falling back to plain
/// scan recency once nothing is enriched yet (e.g. right after a library's
/// very first scan, before any provider is configured) rather than leaving
/// the banner empty. Postgres orders `false < true`, so `DESC` on the
/// "has an overview" test puts an enriched row first without a `CASE`.
///
/// Reads from the `home_candidates` CTE above (see its doc comment) — a show can
/// feature here exactly like a movie can, represented by its own most
/// recently-added episode.
pub async fn get_hero_item(pool: &PgPool) -> Result<Option<MediaItem>, DbError> {
    let row = sqlx::query_as::<_, MediaItemRow>(
        r#"
        WITH home_candidates AS (
            SELECT id, library_id, item_type, title,
                   (metadata->>'year')::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'movie'

            UNION ALL

            SELECT DISTINCT ON (library_id, title)
                   id, library_id, 'series' AS item_type, title,
                   NULL::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'episode'
            ORDER BY library_id, title, created_at DESC, id DESC
        )
        SELECT id, library_id, item_type, title, year, overview, season, episode, created_at
        FROM home_candidates
        ORDER BY (overview IS NOT NULL) DESC, created_at DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(media_item_from_row))
}

/// One item by id, in the same public [`MediaItem`] shape every other
/// reader in this section returns — backs `lisek-server`'s public (any
/// signed-in account, not just admins) Movie/Series Detail View endpoint,
/// distinct from `get_media_item_detail` below which is the admin-only
/// editor's fuller, editable shape (`original_title`/`locked_fields`/
/// `forced_provider` included, and which still addresses one literal
/// episode row by its own id — the editor manages individual scanned
/// files, unaffected by this round's change). `None` covers "no such id at
/// all" and "the id exists but isn't a `media_items` row".
///
/// Runda 8: a bare `episode` row is never handed back as itself any more —
/// nothing in the frontend can even navigate to one after the home-feed
/// collapse above, but an old bookmark/copied link to one still can. Such
/// an id now resolves to that show's own canonical "series" view instead —
/// the exact same representative-episode pick the `home_candidates` CTE above
/// uses, scoped to just this row's own `(library_id, title)` group — so
/// every entry point into a show, old or new, lands on the same page
/// rather than a bare, out-of-context single episode.
pub async fn get_media_item(pool: &PgPool, item_id: Uuid) -> Result<Option<MediaItem>, DbError> {
    let Some((item_type, library_id, title)) = resolve_item_group(pool, item_id).await? else {
        return Ok(None);
    };

    if item_type != "episode" {
        let row = sqlx::query_as::<_, MediaItemRow>(
            r#"
            SELECT id, library_id, item_type, title,
                   (metadata->>'year')::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   (metadata->>'season')::int AS season,
                   (metadata->>'episode')::int AS episode,
                   created_at
            FROM media_items
            WHERE id = $1
            "#,
        )
        .bind(item_id)
        .fetch_optional(pool)
        .await?;
        return Ok(row.map(media_item_from_row));
    }

    let row = sqlx::query_as::<_, MediaItemRow>(
        r#"
        SELECT DISTINCT ON (library_id, title)
               id, library_id, 'series' AS item_type, title,
               NULL::int AS year,
               metadata->'provider_data'->>'overview' AS overview,
               NULL::int AS season,
               NULL::int AS episode,
               created_at
        FROM media_items
        WHERE item_type = 'episode' AND library_id = $1 AND title = $2
        ORDER BY library_id, title, created_at DESC, id DESC
        "#,
    )
    .bind(library_id)
    .bind(title)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(media_item_from_row))
}

/// Looks up which `(item_type, library_id, title)` a `media_items` row
/// belongs to — the shared first step behind [`get_media_item`]'s
/// episode-to-series resolution and [`list_series_seasons`]'s own lookup,
/// so both resolve a given id to "its show" the exact same way.
async fn resolve_item_group(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<(String, Uuid, String)>, DbError> {
    let row: Option<(String, Uuid, String)> =
        sqlx::query_as("SELECT item_type, library_id, title FROM media_items WHERE id = $1")
            .bind(item_id)
            .fetch_optional(pool)
            .await?;
    Ok(row)
}

/// One season's episodes, as returned by [`list_series_seasons`].
/// Deliberately thin: see that function's own doc comment for exactly
/// what's real here and what a filename-only scan (no per-episode
/// metadata call to any provider) simply has no way to know.
pub struct SeriesSeason {
    pub season: i32,
    pub episodes: Vec<SeriesEpisode>,
}

pub struct SeriesEpisode {
    pub id: Uuid,
    pub episode: i32,
    /// Set only for a multi-episode file recognized as a range at scan
    /// time (e.g. `S01E01-E02`) — see `ScannedEpisode::end_episode`.
    pub end_episode: Option<i32>,
}

/// Every season (and, within each, every episode) belonging to the same
/// show `item_id` resolves to via [`resolve_item_group`] — the "Sezony"
/// section of the Series Detail View. Returns an empty `Vec` rather than an
/// error both when `item_id` doesn't exist and when it names a movie: a
/// movie legitimately has no seasons, and the frontend only ever calls this
/// for an item whose own `item_type` it already knows is `"series"`, so
/// there's no case where an empty result here is a surprise worth a
/// distinct error variant.
///
/// Deliberately NOT a per-season lazy-loaded endpoint (`Docelowo,
/// kliknięcie w kartę Sezonu powinno rozwijać... listę odcinków` — one
/// call, fetching every season's episodes up front, with the "expand on
/// click" behavior asked for done client-side against data already in
/// memory. A self-hosted personal library's shows run to tens of episodes,
/// not thousands — the round trip this saves per season click was judged
/// worth more than the (here, negligible) bytes saved by paging it.
///
/// Honest about what an episode "is" here: `id` (for a poster/backdrop/logo
/// URL — see the comment above the `home_candidates` CTE on why every
/// episode row already has its own valid cached images even though
/// they're the show's, not this specific episode's) plus its
/// `season`/`episode` numbers. NOT
/// included, because none of it exists anywhere in this app yet: a real
/// per-episode title (the scanner only ever parses `series_title` — see
/// `lisek_library::ScannedEpisode` — never an episode's own name; that
/// needs a provider's `/episodes` endpoint, never called), a still/
/// thumbnail distinct from the show's own backdrop, a runtime (no video
/// ever gets probed for its duration), or a synopsis (`overview` is
/// series-level, from `EnrichTarget::Series`, identical for every episode
/// of the show). Showing any of those would mean inventing them — the
/// frontend labels each episode generically ("Odcinek {n}") instead.
/// Raw row shape for [`list_series_seasons`]'s query: `(season, id,
/// episode, end_episode)`, all nullable except `id` since every value here
/// comes from the same possibly-sparse `metadata` JSONB extraction as
/// [`MediaItemRow`] above.
type SeriesEpisodeRow = (Option<i32>, Uuid, Option<i32>, Option<i32>);

pub async fn list_series_seasons(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Vec<SeriesSeason>, DbError> {
    let Some((item_type, library_id, title)) = resolve_item_group(pool, item_id).await? else {
        return Ok(Vec::new());
    };
    if item_type != "episode" {
        return Ok(Vec::new());
    }

    let rows: Vec<SeriesEpisodeRow> = sqlx::query_as(
        r#"
        SELECT (metadata->>'season')::int AS season,
               id,
               (metadata->>'episode')::int AS episode,
               (metadata->>'end_episode')::int AS end_episode
        FROM media_items
        WHERE item_type = 'episode' AND library_id = $1 AND title = $2
        ORDER BY (metadata->>'season')::int NULLS LAST, (metadata->>'episode')::int NULLS LAST
        "#,
    )
    .bind(library_id)
    .bind(&title)
    .fetch_all(pool)
    .await?;

    let mut seasons: Vec<SeriesSeason> = Vec::new();
    for (season, id, episode, end_episode) in rows {
        // `season`/`episode` are written unconditionally by
        // `persist_scan_results` for every episode row — `None` here would
        // mean a row this function has no business treating as an episode
        // in the first place, so it's skipped rather than guessed at.
        let (Some(season), Some(episode)) = (season, episode) else {
            continue;
        };
        match seasons.last_mut() {
            Some(s) if s.season == season => s.episodes.push(SeriesEpisode {
                id,
                episode,
                end_episode,
            }),
            _ => seasons.push(SeriesSeason {
                season,
                episodes: vec![SeriesEpisode {
                    id,
                    episode,
                    end_episode,
                }],
            }),
        }
    }
    Ok(seasons)
}

/// The "new" badge shown on a series' home-feed card — see
/// [`classify_series_badge`]'s own doc comment for exactly what each
/// variant means and how it's decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesBadge {
    NewSeries,
    NewSeason,
    NewEpisodes,
}

impl SeriesBadge {
    pub fn as_str(self) -> &'static str {
        match self {
            SeriesBadge::NewSeries => "new_series",
            SeriesBadge::NewSeason => "new_season",
            SeriesBadge::NewEpisodes => "new_episodes",
        }
    }
}

/// A fixed 7-day "just added" window — arbitrary but documented, the same
/// kind of judgment call as `Carousel.svelte`'s `DRAG_THRESHOLD_PX`: there's
/// no scan-batch id anywhere in this schema to say precisely which episodes
/// arrived together, so recency by `created_at` is the honest proxy for it.
const SERIES_BADGE_RECENT_DAYS: i64 = 7;

/// Classifies what changed most recently for one show, for the "Nowy
/// serial" / "Nowy sezon" / "Nowe odcinki" badge asked for on its home-feed
/// card. Computed as its own small query per series-typed card actually
/// being returned (at most 14 per home-feed row — `lisek-server`'s own
/// `ROW_LIMIT`) rather than folded into the `home_candidates` CTE itself —
/// that CTE is already duplicated in full at five call sites because of
/// sqlx's literal-per-call-site rule; adding a nested-`EXISTS` `CASE` to
/// all five, instead of one small function called after, would multiply
/// that same duplication risk for a feature the request itself only asked
/// for "if it can be pulled out" (`Jeśli z API/bazy można to wyciągnąć`).
///
///  - [`SeriesBadge::NewSeries`]: every episode of this show is within the
///    recent window — this show didn't exist in the library at all before
///    now, not even a season.
///  - [`SeriesBadge::NewSeason`]: some episodes are older than the window,
///    and at least one recent episode belongs to a season number no older
///    episode has — a season new to the library, not just new files inside
///    a season already there.
///  - [`SeriesBadge::NewEpisodes`]: recent episodes exist, but only within
///    season numbers already represented among the older ones.
///  - `None`: nothing for this show was added within the window.
pub async fn classify_series_badge(
    pool: &PgPool,
    library_id: Uuid,
    title: &str,
) -> Result<Option<SeriesBadge>, DbError> {
    let rows: Vec<(bool, Option<i32>)> = sqlx::query_as(
        r#"
        SELECT created_at >= now() - make_interval(days => $3), (metadata->>'season')::int
        FROM media_items
        WHERE item_type = 'episode' AND library_id = $1 AND title = $2
        "#,
    )
    .bind(library_id)
    .bind(title)
    .bind(SERIES_BADGE_RECENT_DAYS as i32)
    .fetch_all(pool)
    .await?;

    Ok(classify_badge_from_flags(&rows))
}

/// The pure classification logic behind [`classify_series_badge`], split
/// out so it's testable without a database — each tuple is one episode
/// row's `(is_recent, season)`.
fn classify_badge_from_flags(rows: &[(bool, Option<i32>)]) -> Option<SeriesBadge> {
    use std::collections::HashSet;

    let mut old_seasons: HashSet<i32> = HashSet::new();
    let mut new_seasons: HashSet<i32> = HashSet::new();
    let mut has_old = false;
    let mut has_new = false;

    for &(is_recent, season) in rows {
        if is_recent {
            has_new = true;
            if let Some(s) = season {
                new_seasons.insert(s);
            }
        } else {
            has_old = true;
            if let Some(s) = season {
                old_seasons.insert(s);
            }
        }
    }

    if !has_new {
        return None;
    }
    if !has_old {
        return Some(SeriesBadge::NewSeries);
    }
    if new_seasons.difference(&old_seasons).next().is_some() {
        return Some(SeriesBadge::NewSeason);
    }
    Some(SeriesBadge::NewEpisodes)
}

/// The most recently scanned-or-rescanned items, newest first — the one row
/// with a genuinely unambiguous, non-placeholder ordering. Reads from
/// the `home_candidates` CTE above — a show's `created_at` here is its most
/// recently-added episode's, so a new episode of a show already in the
/// library bumps it back to the top exactly like a freshly-scanned movie.
pub async fn list_recently_added(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        WITH home_candidates AS (
            SELECT id, library_id, item_type, title,
                   (metadata->>'year')::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'movie'

            UNION ALL

            SELECT DISTINCT ON (library_id, title)
                   id, library_id, 'series' AS item_type, title,
                   NULL::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'episode'
            ORDER BY library_id, title, created_at DESC, id DESC
        )
        SELECT id, library_id, item_type, title, year, overview, season, episode, created_at
        FROM home_candidates
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
/// to be a trending chart it has no data to back up. Reads from
/// the `home_candidates` CTE above — a series row's `year` is always `NULL` (no
/// single year describes a whole show, and `ScannedEpisode` never captures
/// one anyway), so shows sort after every dated movie here rather than
/// being given a made-up year just to rank them.
pub async fn list_top_today(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        WITH home_candidates AS (
            SELECT id, library_id, item_type, title,
                   (metadata->>'year')::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'movie'

            UNION ALL

            SELECT DISTINCT ON (library_id, title)
                   id, library_id, 'series' AS item_type, title,
                   NULL::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'episode'
            ORDER BY library_id, title, created_at DESC, id DESC
        )
        SELECT id, library_id, item_type, title, year, overview, season, episode, created_at
        FROM home_candidates
        ORDER BY year DESC NULLS LAST, created_at DESC
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
/// evaluates `random()` fresh per query. Reads from the `home_candidates` CTE above.
pub async fn list_recommended(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        WITH home_candidates AS (
            SELECT id, library_id, item_type, title,
                   (metadata->>'year')::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'movie'

            UNION ALL

            SELECT DISTINCT ON (library_id, title)
                   id, library_id, 'series' AS item_type, title,
                   NULL::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'episode'
            ORDER BY library_id, title, created_at DESC, id DESC
        )
        SELECT id, library_id, item_type, title, year, overview, season, episode, created_at
        FROM home_candidates
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
/// honest random sample rather than a fabricated popularity rank. Reads
/// from the `home_candidates` CTE above.
pub async fn list_popular(pool: &PgPool, limit: i64) -> Result<Vec<MediaItem>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemRow>(
        r#"
        WITH home_candidates AS (
            SELECT id, library_id, item_type, title,
                   (metadata->>'year')::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'movie'

            UNION ALL

            SELECT DISTINCT ON (library_id, title)
                   id, library_id, 'series' AS item_type, title,
                   NULL::int AS year,
                   metadata->'provider_data'->>'overview' AS overview,
                   NULL::int AS season,
                   NULL::int AS episode,
                   created_at
            FROM media_items
            WHERE item_type = 'episode'
            ORDER BY library_id, title, created_at DESC, id DESC
        )
        SELECT id, library_id, item_type, title, year, overview, season, episode, created_at
        FROM home_candidates
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

    // Runda 8: `classify_badge_from_flags` is the pure decision logic
    // behind `classify_series_badge` — split out specifically so it's
    // testable without a live Postgres, same reasoning as
    // `settings_has_api_key` above. `true`/`false` stands in for "this
    // episode's `created_at` falls inside the recent window or not"; the
    // `Option<i32>` is its season number.

    #[test]
    fn badge_is_new_series_when_every_episode_is_recent() {
        let rows = [(true, Some(1)), (true, Some(1)), (true, Some(2))];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewSeries)
        );
    }

    #[test]
    fn badge_is_none_when_nothing_is_recent() {
        let rows = [(false, Some(1)), (false, Some(1))];
        assert_eq!(classify_badge_from_flags(&rows), None);
    }

    #[test]
    fn badge_is_new_season_when_a_recent_episode_has_an_unseen_season_number() {
        // Season 1 already existed; season 2 just arrived.
        let rows = [(false, Some(1)), (false, Some(1)), (true, Some(2))];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewSeason)
        );
    }

    #[test]
    fn badge_is_new_episodes_when_recent_episodes_stay_within_a_known_season() {
        // Season 1 already existed and gained two more recent episodes.
        let rows = [(false, Some(1)), (true, Some(1)), (true, Some(1))];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewEpisodes)
        );
    }

    #[test]
    fn badge_ignores_a_missing_season_number_rather_than_panicking() {
        // Defensive only — every real episode row always has a season
        // (see `persist_scan_results`), but this must never crash on one
        // that somehow doesn't.
        let rows = [(true, None)];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewSeries)
        );
    }
}
