//! Admin-only endpoints: the metadata provider registry backing the Admin
//! Panel's plugins tab (enable/disable, priority, and setting/clearing an
//! API key), plus, as of Faza 5, library management (create/list/delete)
//! and the scan-trigger endpoint that runs `lisek-library`'s scanner over
//! one library's root paths and persists the result. Every handler here
//! takes `AuthUser` and immediately calls `require_admin` — that check (not
//! a separate role-checking middleware layer) is this module's entire
//! security boundary, the same way `AuthUser` itself is `auth.rs`'s.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::metadata_enrichment;
use crate::state::SharedState;

#[derive(Serialize)]
pub struct ProviderConfigResponse {
    pub provider_key: String,
    pub display_name: String,
    pub enabled: bool,
    pub priority: i32,
    pub is_configured: bool,
}

impl From<lisek_metadata::ProviderConfig> for ProviderConfigResponse {
    fn from(config: lisek_metadata::ProviderConfig) -> Self {
        Self {
            provider_key: config.provider_key,
            display_name: config.display_name,
            enabled: config.enabled,
            priority: config.priority,
            is_configured: config.is_configured,
        }
    }
}

pub async fn list_providers(
    auth: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<ProviderConfigResponse>>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let providers = lisek_db::list_provider_configs(&pool)
        .await
        .map_err(|_| AdminError::Internal)?;

    Ok(Json(providers.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize)]
pub struct UpdateProviderRequest {
    pub enabled: bool,
    pub priority: i32,
}

pub async fn update_provider(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(provider_key): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<Json<Value>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    lisek_db::set_provider_enabled_and_priority(&pool, &provider_key, req.enabled, req.priority)
        .await
        .map_err(map_db_error)?;

    tracing::info!(
        provider_key,
        enabled = req.enabled,
        priority = req.priority,
        admin = %auth.user.username,
        "provider config updated"
    );

    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SetProviderKeyRequest {
    pub api_key: String,
}

/// Sets a provider's API key. Deliberately has no matching "get the current
/// key" endpoint — [`list_providers`]'s `is_configured` flag is the only
/// thing a client ever learns about a stored key without setting a new one.
pub async fn set_provider_key(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(provider_key): Path<String>,
    Json(req): Json<SetProviderKeyRequest>,
) -> Result<Json<Value>, AdminError> {
    require_admin(&auth)?;

    let api_key = req.api_key.trim();
    if api_key.is_empty() {
        return Err(AdminError::InvalidInput(
            "api_key cannot be empty".to_string(),
        ));
    }

    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;
    lisek_db::set_provider_api_key(&pool, &provider_key, api_key)
        .await
        .map_err(map_db_error)?;

    tracing::info!(provider_key, admin = %auth.user.username, "provider API key set");

    Ok(Json(json!({ "ok": true })))
}

pub async fn clear_provider_key(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(provider_key): Path<String>,
) -> Result<Json<Value>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    lisek_db::clear_provider_api_key(&pool, &provider_key)
        .await
        .map_err(map_db_error)?;

    tracing::info!(provider_key, admin = %auth.user.username, "provider API key cleared");

    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize)]
pub struct LibraryResponse {
    pub id: Uuid,
    pub name: String,
    pub library_type: String,
    pub sort_order: i32,
    pub root_paths: Vec<String>,
    pub save_metadata_in_media_folder: bool,
}

impl From<lisek_core::Library> for LibraryResponse {
    fn from(library: lisek_core::Library) -> Self {
        Self {
            id: library.id,
            name: library.name,
            library_type: library.library_type,
            sort_order: library.sort_order,
            root_paths: library.root_paths,
            save_metadata_in_media_folder: library.save_metadata_in_media_folder,
        }
    }
}

pub async fn list_libraries(
    auth: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<LibraryResponse>>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let libraries = lisek_db::list_libraries(&pool)
        .await
        .map_err(map_db_error)?;
    Ok(Json(libraries.into_iter().map(Into::into).collect()))
}

/// Most self-hosters expect provider-downloaded images to live alongside
/// their media rather than only in an app-private cache, and it keeps a
/// library portable to another tool later — so a
/// library-creation request that omits this field entirely (an older
/// frontend build, a direct API call) still gets the behavior the roadmap
/// and the Faza 5 round-2 migration both settled on as the default.
fn default_save_metadata_in_media_folder() -> bool {
    true
}

#[derive(Deserialize)]
pub struct CreateLibraryRequest {
    pub name: String,
    pub library_type: String,
    pub root_paths: Vec<String>,
    #[serde(default = "default_save_metadata_in_media_folder")]
    pub save_metadata_in_media_folder: bool,
}

pub async fn create_library(
    auth: AuthUser,
    State(state): State<SharedState>,
    Json(req): Json<CreateLibraryRequest>,
) -> Result<Json<LibraryResponse>, AdminError> {
    require_admin(&auth)?;

    let name = req.name.trim();
    if name.is_empty() {
        return Err(AdminError::InvalidInput("name cannot be empty".to_string()));
    }
    if req.root_paths.is_empty() {
        return Err(AdminError::InvalidInput(
            "at least one root path is required".to_string(),
        ));
    }

    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;
    let library = lisek_db::create_library(
        &pool,
        name,
        &req.library_type,
        &req.root_paths,
        req.save_metadata_in_media_folder,
    )
    .await
    .map_err(map_db_error)?;

    tracing::info!(
        library_id = %library.id,
        library_type = %library.library_type,
        admin = %auth.user.username,
        "library created"
    );

    Ok(Json(library.into()))
}

pub async fn delete_library(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<Value>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    lisek_db::delete_library(&pool, library_id)
        .await
        .map_err(map_db_error)?;

    tracing::info!(%library_id, admin = %auth.user.username, "library deleted");

    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize)]
pub struct ScanSummaryResponse {
    pub movies_found: usize,
    pub episodes_found: usize,
    pub unrecognized: usize,
    /// How many of the items above matched an enabled, configured metadata
    /// provider during this scan's enrichment pass. Always `0` when no
    /// provider is both enabled and has an API key set — see the
    /// `tracing::warn!` this endpoint emits for that exact case — but a low
    /// number here can just as well mean a provider that didn't recognize
    /// some titles, so this is a hint to check the Admin Panel's Plugins
    /// tab, not on its own proof of a misconfiguration.
    pub metadata_matched: usize,
    /// How many items actually got a poster and/or backdrop written to
    /// disk (media folder or local cache) this scan. Can be lower than
    /// `metadata_matched` when a provider matched a title but had no image
    /// for it, or the download/write itself failed — see the per-item
    /// `tracing::warn!` lines `metadata_enrichment::enrich_item` emits for
    /// which one happened.
    pub images_saved: usize,
}

/// Walks one library's root paths on disk (`lisek-library`'s scanner),
/// parses every video file found (movie title/year, or series/season/
/// episode — whichever `library_type` calls for), and upserts the results
/// into `media_items`. Synchronous from the caller's point of view: a large
/// library can make this take a while, which is an acceptable Faza 5
/// simplification (see the roadmap notes on a future background job queue)
/// rather than something this endpoint tries to solve.
pub async fn scan_library(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<ScanSummaryResponse>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let library = lisek_db::get_library(&pool, library_id)
        .await
        .map_err(map_db_error)?
        .ok_or(AdminError::LibraryNotFound)?;

    let library_type =
        lisek_library::LibraryType::from_db_str(&library.library_type).ok_or_else(|| {
            AdminError::InvalidInput("library has an unrecognized library_type".to_string())
        })?;

    let now_year = chrono::Utc::now().year();
    let results = lisek_library::scan_library(&library.root_paths, library_type, now_year);
    let persisted = lisek_db::persist_scan_results(&pool, library_id, &results)
        .await
        .map_err(map_db_error)?;

    // Best-effort metadata enrichment: look up every newly-touched item
    // against whichever providers are enabled and have a key configured,
    // in the admin's priority order (see `metadata_enrichment`). Skipped
    // entirely, with a `warn!` (not silently, and not merely at `debug!` —
    // this is a common real first-run trap: `tmdb`'s `provider_config` row
    // is enabled out of the box, but still needs an admin-supplied API key
    // before it counts as usable), when no provider is usable yet — a scan
    // is still useful with bare filenames-derived titles even so.
    let global_providers = lisek_db::list_enabled_provider_keys(&pool)
        .await
        .map_err(map_db_error)?;
    // A library can override the admin's global priority order for its own
    // item type (the Admin Panel's drag-and-drop reordering — see
    // `lisek_db::get_library_provider_order`'s doc comment). `None` (no
    // override ever saved for this library) falls back to the global order
    // unchanged; when one exists, `apply_provider_order` keeps only the
    // providers that are still actually enabled/configured, in the
    // library's chosen order — a provider the admin disabled since saving
    // that order simply drops out rather than erroring the whole scan.
    let library_item_type = item_type_for_library(&library.library_type);
    let library_order = lisek_db::get_library_provider_order(&pool, library_id, library_item_type)
        .await
        .map_err(map_db_error)?;
    let providers = apply_provider_order(global_providers, library_order.as_deref());

    let mut metadata_matched = 0usize;
    let mut images_saved = 0usize;
    if providers.is_empty() {
        tracing::warn!(
            %library_id,
            "no enabled/configured metadata provider — skipping enrichment for this scan; \
             set an API key for at least one provider under Admin Panel \u{2192} Plugins"
        );
    } else {
        let http = reqwest::Client::new();
        let config_dir = crate::state::config_dir();
        // No dedicated server-wide "metadata language" setting exists yet
        // (see the roadmap's provider-language-priority item) — the
        // scanning admin's own UI language is a reasonable, honest stand-in
        // for "this server's primary language" until that's built.
        let primary_language = auth.user.ui_language.as_str();
        for (item_id, target, file_path, forced_provider) in &persisted.enrich_targets {
            let outcome = metadata_enrichment::enrich_item(
                &pool,
                &http,
                &config_dir,
                &providers,
                *item_id,
                target,
                file_path,
                library.save_metadata_in_media_folder,
                primary_language,
                forced_provider.as_deref(),
            )
            .await;
            if outcome.matched {
                metadata_matched += 1;
            }
            if outcome.image_saved {
                images_saved += 1;
            }
        }
    }

    tracing::info!(
        %library_id,
        movies_found = persisted.summary.movies_found,
        episodes_found = persisted.summary.episodes_found,
        unrecognized = persisted.summary.unrecognized,
        metadata_matched,
        images_saved,
        admin = %auth.user.username,
        "library scan completed"
    );

    Ok(Json(ScanSummaryResponse {
        movies_found: persisted.summary.movies_found,
        episodes_found: persisted.summary.episodes_found,
        unrecognized: persisted.summary.unrecognized,
        metadata_matched,
        images_saved,
    }))
}

/// Every item in one library, for the Admin Panel's browse view that opens
/// the metadata editor (see `lisek_db::MediaItemSummary`'s own doc comment
/// for exactly what rides along).
pub async fn list_library_items(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<Vec<lisek_db::MediaItemSummary>>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let items = lisek_db::list_media_items_for_library(&pool, library_id)
        .await
        .map_err(map_db_error)?;
    Ok(Json(items))
}

/// One item's full editable shape, for the metadata editor to load when
/// it opens.
pub async fn get_media_item(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<lisek_db::MediaItemDetail>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let item = lisek_db::get_media_item_detail(&pool, item_id)
        .await
        .map_err(map_db_error)?
        .ok_or(AdminError::ItemNotFound)?;
    Ok(Json(item))
}

#[derive(Deserialize)]
pub struct UpdateMediaItemRequest {
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub metadata_locked: bool,
    pub forced_provider: Option<String>,
    pub locked_fields: Vec<String>,
}

/// The metadata editor's save action — see `lisek_db::update_media_item`'s
/// own doc comment for why this is the one write in the whole admin
/// surface that's never gated on `locked_fields` (an admin's own edit
/// always applies) and the one place `locked_fields` itself, plus
/// `metadata_locked`/`forced_provider`, actually get written.
pub async fn update_media_item(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<UpdateMediaItemRequest>,
) -> Result<Json<Value>, AdminError> {
    require_admin(&auth)?;

    let title = req.title.trim();
    if title.is_empty() {
        return Err(AdminError::InvalidInput(
            "title cannot be empty".to_string(),
        ));
    }

    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;
    let updated = lisek_db::update_media_item(
        &pool,
        item_id,
        title,
        req.original_title.as_deref(),
        req.year,
        req.overview.as_deref(),
        req.metadata_locked,
        req.forced_provider.as_deref(),
        &req.locked_fields,
    )
    .await
    .map_err(map_db_error)?;

    if !updated {
        return Err(AdminError::ItemNotFound);
    }

    tracing::info!(
        %item_id,
        metadata_locked = req.metadata_locked,
        forced_provider = req.forced_provider.as_deref(),
        admin = %auth.user.username,
        "media item metadata updated"
    );

    Ok(Json(json!({ "ok": true })))
}

/// Every item currently flagged `metadata_fallback` — the Admin Panel's
/// warning view.
pub async fn list_fallback_items(
    auth: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<lisek_db::FallbackItem>>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let items = lisek_db::list_fallback_items(&pool)
        .await
        .map_err(map_db_error)?;
    Ok(Json(items))
}

#[derive(Serialize)]
pub struct MigrateFallbackResponse {
    pub poster_migrated: bool,
    pub backdrop_migrated: bool,
}

/// The fallback warning view's "Przenieś do folderu mediów" action: copies
/// whichever of this item's poster/backdrop currently only exist in the
/// local `config_dir` cache into its media folder (right next to the
/// video file, same `poster.jpg`/`backdrop.jpg` naming
/// `metadata_enrichment::enrich_item` already uses), then clears
/// `metadata_fallback` once at least one image actually made it across —
/// a partial success (say, the destination folder only just got its write
/// permission fixed for one of the two files) still counts as real
/// progress worth reflecting in the flag.
pub async fn migrate_fallback_item(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<MigrateFallbackResponse>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let file_path = lisek_db::get_media_item_file_path(&pool, item_id)
        .await
        .map_err(map_db_error)?
        .ok_or(AdminError::ItemNotFound)?;
    let Some(parent) = std::path::Path::new(&file_path).parent() else {
        return Err(AdminError::InvalidInput(
            "this item's file_path has no parent directory".to_string(),
        ));
    };

    let config_dir = crate::state::config_dir();
    let poster_migrated = copy_cached_image(
        &lisek_metadata::image_cache::poster_cache_path(&config_dir, item_id),
        &parent.join("poster.jpg"),
    )
    .await;
    let backdrop_migrated = copy_cached_image(
        &lisek_metadata::image_cache::backdrop_cache_path(&config_dir, item_id),
        &parent.join("backdrop.jpg"),
    )
    .await;

    if poster_migrated || backdrop_migrated {
        lisek_db::set_metadata_fallback(&pool, item_id, false)
            .await
            .map_err(map_db_error)?;
    }

    tracing::info!(
        %item_id,
        poster_migrated,
        backdrop_migrated,
        admin = %auth.user.username,
        "migrated cached image(s) to the media folder"
    );

    Ok(Json(MigrateFallbackResponse {
        poster_migrated,
        backdrop_migrated,
    }))
}

/// Copies one cached image into the media folder, best-effort: `false`
/// covers both "nothing to migrate" (no cached copy exists — this item
/// never fell back for this particular image) and a genuine write
/// failure (logged here, not surfaced as this whole request's error — a
/// poster succeeding while a backdrop's destination is still unwritable
/// is still a partial win worth reporting via [`MigrateFallbackResponse`]
/// rather than failing the whole action).
async fn copy_cached_image(cache_path: &std::path::Path, dest_path: &std::path::Path) -> bool {
    match tokio::fs::read(cache_path).await {
        Ok(bytes) => match tokio::fs::write(dest_path, &bytes).await {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!(
                    cache_path = %cache_path.display(),
                    dest_path = %dest_path.display(),
                    %err,
                    "failed to write migrated image to the media folder"
                );
                false
            }
        },
        Err(_) => false,
    }
}

#[derive(Serialize, Deserialize)]
pub struct ProviderOrderPayload {
    pub provider_order: Vec<String>,
}

/// A library's saved custom provider order for one item type
/// (`"movie"`/`"episode"`) — the Admin Panel's drag-and-drop reordering UI.
/// An empty `provider_order` (never saved, or explicitly cleared) means
/// "use the admin's global priority order" — see `apply_provider_order`.
pub async fn get_provider_order(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path((library_id, item_type)): Path<(Uuid, String)>,
) -> Result<Json<ProviderOrderPayload>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    let order = lisek_db::get_library_provider_order(&pool, library_id, &item_type)
        .await
        .map_err(map_db_error)?
        .unwrap_or_default();
    Ok(Json(ProviderOrderPayload {
        provider_order: order,
    }))
}

pub async fn set_provider_order(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path((library_id, item_type)): Path<(Uuid, String)>,
    Json(req): Json<ProviderOrderPayload>,
) -> Result<Json<Value>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;

    lisek_db::set_library_provider_order(&pool, library_id, &item_type, &req.provider_order)
        .await
        .map_err(map_db_error)?;

    tracing::info!(
        %library_id,
        item_type,
        provider_order = ?req.provider_order,
        admin = %auth.user.username,
        "library provider order updated"
    );

    Ok(Json(json!({ "ok": true })))
}

/// Maps a library's own type (`"movies"`/`"tv_shows"`,
/// `lisek_library::LibraryType`'s db strings) to the item_type vocabulary
/// `media_items` and `library_provider_config` both actually store
/// (`"movie"`/`"episode"`) — two different naming schemes for the same
/// distinction, kept separate on purpose (a library IS a type; an item HAS
/// a type) rather than unifying them prematurely.
pub(crate) fn item_type_for_library(library_type: &str) -> &'static str {
    if library_type == "movies" {
        "movie"
    } else {
        "episode"
    }
}

/// Reorders (and filters) `global` — the admin's globally enabled/
/// configured `(provider_key, api_key)` list, in the admin's own priority
/// order — to match `custom_order` (a library's saved provider_order, key
/// names only) when one is given and non-empty. A key in `custom_order`
/// that isn't in `global` (disabled, or never configured) is silently
/// skipped rather than erroring; a `global` entry with no matching key in
/// `custom_order` (a provider enabled since the library's order was last
/// saved) is appended after every explicitly ordered one, so a newly
/// enabled provider still gets tried for this library rather than
/// silently never running until the admin revisits its order.
pub(crate) fn apply_provider_order(
    global: Vec<(String, String)>,
    custom_order: Option<&[String]>,
) -> Vec<(String, String)> {
    let Some(custom_order) = custom_order.filter(|order| !order.is_empty()) else {
        return global;
    };

    let mut remaining = global;
    let mut ordered = Vec::with_capacity(remaining.len());
    for key in custom_order {
        if let Some(pos) = remaining.iter().position(|(k, _)| k == key) {
            ordered.push(remaining.remove(pos));
        }
    }
    ordered.extend(remaining);
    ordered
}

fn require_admin(auth: &AuthUser) -> Result<(), AdminError> {
    if auth.user.is_admin {
        Ok(())
    } else {
        Err(AdminError::Forbidden)
    }
}

fn map_db_error(err: lisek_db::DbError) -> AdminError {
    match err {
        lisek_db::DbError::ProviderNotFound => AdminError::ProviderNotFound,
        lisek_db::DbError::LibraryNotFound => AdminError::LibraryNotFound,
        lisek_db::DbError::InvalidLibraryType(value) => AdminError::InvalidInput(format!(
            "'{value}' is not a valid library type (expected \"movies\" or \"tv_shows\")"
        )),
        _ => AdminError::Internal,
    }
}

#[derive(Debug)]
pub enum AdminError {
    ServerNotReady,
    Forbidden,
    ProviderNotFound,
    LibraryNotFound,
    ItemNotFound,
    InvalidInput(String),
    Internal,
}

impl IntoResponse for AdminError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            AdminError::ServerNotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "server_not_ready",
                "The server isn't fully configured yet.".to_string(),
            ),
            AdminError::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "This account is not an administrator.".to_string(),
            ),
            AdminError::ProviderNotFound => (
                StatusCode::NOT_FOUND,
                "provider_not_found",
                "No such metadata provider is registered.".to_string(),
            ),
            AdminError::LibraryNotFound => (
                StatusCode::NOT_FOUND,
                "library_not_found",
                "No such library is configured.".to_string(),
            ),
            AdminError::ItemNotFound => (
                StatusCode::NOT_FOUND,
                "item_not_found",
                "No such media item exists.".to_string(),
            ),
            AdminError::InvalidInput(message) => {
                (StatusCode::BAD_REQUEST, "invalid_input", message)
            }
            AdminError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Something went wrong while updating that setting.".to_string(),
            ),
        };

        (
            status,
            Json(json!({ "error_code": error_code, "message": message })),
        )
            .into_response()
    }
}
