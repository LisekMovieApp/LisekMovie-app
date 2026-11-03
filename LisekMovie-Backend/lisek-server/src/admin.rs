//! Admin-only endpoints: the metadata provider registry backing the Admin
//! Panel's plugins tab (enable/disable, priority, and setting/clearing an
//! API key), plus, as of Faza 5, library management (create/list/delete).
//! Scans are queued from `scan_jobs.rs` and run by its background worker.
//!
//! Access: every route here is registered in the admin router (`routes.rs`),
//! which puts the `AdminUser` extractor in front of all of them as a layer.
//! Each handler also takes `AdminUser` itself, so moving a handler to the
//! wrong router cannot silently make it public.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::AdminUser;
use crate::state::SharedState;

#[derive(Serialize, utoipa::ToSchema)]
pub struct ProviderConfigResponse {
    pub provider_key: String,
    pub display_name: String,
    pub enabled: bool,
    pub priority: i32,
    /// A key is stored, or the provider works without one.
    pub is_configured: bool,
    /// A key is stored (the key itself is never returned).
    pub key_stored: bool,
    /// `required`, `optional` or `none`.
    pub key_requirement: String,
    /// What the provider has data for: `movies`, `series`, `anime`,
    /// `music_videos`, `music`.
    pub covers: Vec<String>,
    /// `full`, `basic` or `skeleton` (registered, no results yet).
    pub stage: String,
    /// Where to get a key or read about the source.
    pub website: Option<String>,
}

impl From<lisek_metadata::ProviderConfig> for ProviderConfigResponse {
    fn from(config: lisek_metadata::ProviderConfig) -> Self {
        Self {
            provider_key: config.provider_key,
            display_name: config.display_name,
            enabled: config.enabled,
            priority: config.priority,
            is_configured: config.is_configured,
            key_stored: config.key_stored,
            key_requirement: config.key_requirement.as_str().to_string(),
            covers: config.covers.iter().map(|c| c.as_str().to_string()).collect(),
            stage: config.stage.as_str().to_string(),
            website: config.website,
        }
    }
}

#[utoipa::path(get,path="/api/admin/providers",tag="admin",responses((status=200,body=Vec<ProviderConfigResponse>),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn list_providers(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<ProviderConfigResponse>>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

    let providers = lisek_db::list_provider_configs(&pool)
        .await
        .map_err(|_| AdminError::Internal)?;

    Ok(Json(providers.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateProviderRequest {
    pub enabled: bool,
    pub priority: i32,
}

#[utoipa::path(put,path="/api/admin/providers/{provider_key}",tag="admin",params(("provider_key"=String,Path)),request_body=UpdateProviderRequest,responses((status=200,body=Value),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn update_provider(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(provider_key): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> Result<Json<Value>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

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

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetProviderKeyRequest {
    pub api_key: String,
}

/// Sets a provider's API key. Deliberately has no matching "get the current
/// key" endpoint — [`list_providers`]'s `is_configured` flag is the only
/// thing a client ever learns about a stored key without setting a new one.
#[utoipa::path(put,path="/api/admin/providers/{provider_key}/key",tag="admin",params(("provider_key"=String,Path)),request_body=SetProviderKeyRequest,responses((status=200,body=Value),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn set_provider_key(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(provider_key): Path<String>,
    Json(req): Json<SetProviderKeyRequest>,
) -> Result<Json<Value>, AdminError> {
    let api_key = req.api_key.trim();
    if api_key.is_empty() {
        return Err(AdminError::InvalidInput(
            "api_key cannot be empty".to_string(),
        ));
    }

    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;
    // Sealed with the provider key as context: the database never sees the
    // plaintext, and a value copied to another provider's row won't open.
    let sealed = state.secrets.seal(api_key, &provider_key);
    lisek_db::set_provider_api_key(&pool, &provider_key, &sealed)
        .await
        .map_err(map_db_error)?;

    tracing::info!(provider_key, admin = %auth.user.username, "provider API key set");

    Ok(Json(json!({ "ok": true })))
}

#[utoipa::path(delete,path="/api/admin/providers/{provider_key}/key",tag="admin",params(("provider_key"=String,Path)),responses((status=200,body=Value),(status=404),(status=401),(status=403),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn clear_provider_key(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(provider_key): Path<String>,
) -> Result<Json<Value>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

    lisek_db::clear_provider_api_key(&pool, &provider_key)
        .await
        .map_err(map_db_error)?;

    tracing::info!(provider_key, admin = %auth.user.username, "provider API key cleared");

    Ok(Json(json!({ "ok": true })))
}

#[derive(Serialize, utoipa::ToSchema)]
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

#[utoipa::path(get,path="/api/admin/libraries",tag="admin",responses((status=200,body=Vec<LibraryResponse>),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn list_libraries(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<LibraryResponse>>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

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

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateLibraryRequest {
    pub name: String,
    pub library_type: String,
    pub root_paths: Vec<String>,
    #[serde(default = "default_save_metadata_in_media_folder")]
    pub save_metadata_in_media_folder: bool,
    /// Faza 20, `live_tv` only: the M3U channel list (http(s) or a path on
    /// the server) and the optional XMLTV guide. `root_paths` stays empty.
    #[serde(default)]
    pub m3u_url: Option<String>,
    #[serde(default)]
    pub xmltv_url: Option<String>,
}

#[utoipa::path(post,path="/api/admin/libraries",tag="admin",request_body=CreateLibraryRequest,responses((status=200,body=LibraryResponse),(status=400),(status=401),(status=403),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn create_library(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Json(req): Json<CreateLibraryRequest>,
) -> Result<Json<LibraryResponse>, AdminError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(AdminError::InvalidInput("name cannot be empty".to_string()));
    }
    let live_tv = req.library_type == lisek_db::livetv::LIVE_TV;
    let source = if live_tv {
        let source = crate::livetv::SetSource {
            m3u_url: req.m3u_url.clone().unwrap_or_default(),
            xmltv_url: req.xmltv_url.clone(),
        };
        crate::livetv::check_source(&source)
            .map_err(|_| AdminError::InvalidInput("a valid M3U link or server path is required".to_string()))?;
        Some(source)
    } else {
        None
    };
    // Faza 25: folders are stored normalized (absolute, no trailing slash).
    let root_paths = if live_tv { Vec::new() } else { clean_roots(&req.root_paths)? };
    if !live_tv && root_paths.is_empty() {
        return Err(AdminError::InvalidInput(
            "at least one root path is required".to_string(),
        ));
    }

    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;
    let library = lisek_db::create_library(
        &pool,
        name,
        &req.library_type,
        &root_paths,
        req.save_metadata_in_media_folder,
    )
    .await
    .map_err(map_db_error)?;
    if let Some(source) = source {
        let xmltv = source.xmltv_url.as_deref().map(str::trim).filter(|u| !u.is_empty());
        lisek_db::livetv::set_source(&pool, library.id, source.m3u_url.trim(), xmltv)
            .await
            .map_err(map_db_error)?;
        // Channels and guide arrive in the background (big guides take a while).
        let (pool, library_id) = (pool.clone(), library.id);
        tokio::spawn(async move {
            let _ = crate::livetv::refresh(&pool, library_id).await;
        });
    }

    tracing::info!(
        library_id = %library.id,
        library_type = %library.library_type,
        admin = %auth.user.username,
        "library created"
    );

    Ok(Json(library.into()))
}

/// Faza 25: what the "Edytuj" modal sends. Fields left out stay as they are.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateLibraryRequest {
    pub name: Option<String>,
    /// Every folder the library should scan (absolute paths). Items inside
    /// a removed folder leave the library (the files stay on disk).
    pub root_paths: Option<Vec<String>>,
    pub save_metadata_in_media_folder: Option<bool>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UpdateLibraryResponse {
    pub library: LibraryResponse,
    /// Items that left the library with a removed folder.
    pub removed_items: u64,
    /// Folders the server cannot see right now (not mounted yet, a typo):
    /// saved anyway, scanned once they exist.
    pub missing_paths: Vec<String>,
}

/// Most folders one library may have.
const MAX_ROOT_PATHS: usize = 32;

/// Folders as typed: normalized, without duplicates. `Err` names the
/// first unusable one.
fn clean_roots(paths: &[String]) -> Result<Vec<String>, AdminError> {
    let mut out: Vec<String> = Vec::new();
    for path in paths {
        if path.trim().is_empty() {
            continue;
        }
        let root = lisek_db::normalize_root(path).ok_or_else(|| {
            AdminError::InvalidInput(format!("not an absolute folder path: {}", path.trim()))
        })?;
        if !out.contains(&root) {
            out.push(root);
        }
    }
    if out.len() > MAX_ROOT_PATHS {
        return Err(AdminError::InvalidInput(format!("at most {MAX_ROOT_PATHS} folders")));
    }
    Ok(out)
}

/// Renames a library and changes its folders ("Edytuj" on the libraries
/// tab). A live TV library only has a name here; its sources have their
/// own panel.
#[utoipa::path(put,path="/api/admin/libraries/{library_id}",tag="admin",params(("library_id"=uuid::Uuid,Path)),request_body=UpdateLibraryRequest,responses((status=200,body=UpdateLibraryResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn update_library(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
    Json(req): Json<UpdateLibraryRequest>,
) -> Result<Json<UpdateLibraryResponse>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;
    let current = lisek_db::get_library(&pool, library_id)
        .await
        .map_err(map_db_error)?
        .ok_or(AdminError::LibraryNotFound)?;
    let name = match &req.name {
        Some(name) => name.trim().to_owned(),
        None => current.name.clone(),
    };
    if name.is_empty() || name.chars().count() > 200 {
        return Err(AdminError::InvalidInput("name cannot be empty".to_string()));
    }
    let live_tv = current.library_type == lisek_db::livetv::LIVE_TV;
    let roots = match (&req.root_paths, live_tv) {
        (Some(_), true) => {
            return Err(AdminError::InvalidInput("a live TV library has no folders".to_string()));
        }
        (Some(paths), false) => clean_roots(paths)?,
        (None, _) => current.root_paths.clone(),
    };
    if !live_tv && roots.is_empty() {
        return Err(AdminError::InvalidInput("at least one root path is required".to_string()));
    }
    let save = req
        .save_metadata_in_media_folder
        .unwrap_or(current.save_metadata_in_media_folder);
    let updated = lisek_db::update_library(&pool, library_id, &name, &roots, save)
        .await
        .map_err(map_db_error)?
        .ok_or(AdminError::LibraryNotFound)?;
    let missing_paths = {
        let roots = roots.clone();
        tokio::task::spawn_blocking(move || {
            roots
                .into_iter()
                .filter(|r| !std::path::Path::new(r).is_dir())
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default()
    };
    tracing::info!(
        %library_id,
        name,
        roots = ?roots,
        removed_items = updated.removed_items,
        missing = ?missing_paths,
        admin = %auth.user.username,
        "library updated"
    );
    if updated.removed_items > 0 {
        let config_dir = crate::state::config_dir();
        let pool = pool.clone();
        tokio::spawn(async move {
            crate::metadata_enrichment::prune_collections(&pool, &config_dir).await;
        });
    }
    Ok(Json(UpdateLibraryResponse {
        library: updated.library.into(),
        removed_items: updated.removed_items,
        missing_paths,
    }))
}

#[utoipa::path(delete,path="/api/admin/libraries/{library_id}",tag="admin",params(("library_id"=uuid::Uuid,Path)),responses((status=200,body=Value),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn delete_library(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<Value>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

    lisek_db::delete_library(&pool, library_id)
        .await
        .map_err(map_db_error)?;

    tracing::info!(%library_id, admin = %auth.user.username, "library deleted");
    // Faza 24: its items' credits went with them; people credited nowhere
    // else (and collections left empty) go now, not at the next scan.
    let config_dir = crate::state::config_dir();
    tokio::spawn(async move {
        crate::metadata_enrichment::prune_collections(&pool, &config_dir).await;
    });

    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct MoveLibraryRequest {
    /// `"up"` or `"down"` — anything else is [`AdminError::InvalidInput`],
    /// same validate-before-any-query discipline as [`create_library`]'s
    /// own `library_type` check.
    pub direction: String,
}

/// Nudges one library up or down in the shared display order — the "W
/// górę"/"W dół" buttons the libraries tab asked for (Runda 9). See
/// `lisek_db::move_library`'s own doc comment for exactly what "up"/"down"
/// do (a swap with the immediate neighbor, not a full reorder) and why this
/// ordering is global rather than per-account.
#[utoipa::path(put,path="/api/admin/libraries/{library_id}/reorder",tag="admin",params(("library_id"=uuid::Uuid,Path)),request_body=MoveLibraryRequest,responses((status=200,body=Value),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn move_library(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
    Json(req): Json<MoveLibraryRequest>,
) -> Result<Json<Value>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

    let direction = match req.direction.as_str() {
        "up" => lisek_db::LibraryMoveDirection::Up,
        "down" => lisek_db::LibraryMoveDirection::Down,
        other => {
            return Err(AdminError::InvalidInput(format!(
                "'{other}' is not a valid direction (expected \"up\" or \"down\")"
            )));
        }
    };

    lisek_db::move_library(&pool, library_id, direction)
        .await
        .map_err(map_db_error)?;

    tracing::info!(
        %library_id,
        direction = %req.direction,
        admin = %auth.user.username,
        "library reordered"
    );

    Ok(Json(json!({ "ok": true })))
}

/// Every item in one library, for the Admin Panel's browse view that opens
/// the metadata editor (see `lisek_db::MediaItemSummary`'s own doc comment
/// for exactly what rides along).
#[utoipa::path(get,path="/api/admin/libraries/{library_id}/items",tag="admin",params(("library_id"=uuid::Uuid,Path)),responses((status=200,body=Vec<lisek_db::MediaItemSummary>),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn list_library_items(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<Vec<lisek_db::MediaItemSummary>>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

    let items = lisek_db::list_media_items_for_library(&pool, library_id)
        .await
        .map_err(map_db_error)?;
    Ok(Json(items))
}

/// Every item currently flagged `metadata_fallback` — the Admin Panel's
/// warning view.
#[utoipa::path(get,path="/api/admin/media/fallback",tag="admin",responses((status=200,body=Vec<lisek_db::FallbackItem>),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn list_fallback_items(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<lisek_db::FallbackItem>>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

    let items = lisek_db::list_fallback_items(&pool)
        .await
        .map_err(map_db_error)?;
    Ok(Json(items))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct MigrateFallbackResponse {
    pub poster_migrated: bool,
    pub backdrop_migrated: bool,
}

/// The fallback warning view's "Przenieś do folderu mediów" action: copies
/// whichever of this item's poster/backdrop currently only exist in the
/// local `config_dir` cache into its media folder (right next to the
/// video file, same `poster.jpg`/`backdrop.jpg` naming
/// `metadata_enrichment::enrich_item_cached` already uses), then clears
/// `metadata_fallback` once at least one image actually made it across —
/// a partial success (say, the destination folder only just got its write
/// permission fixed for one of the two files) still counts as real
/// progress worth reflecting in the flag.
#[utoipa::path(post,path="/api/admin/media/{item_id}/migrate-to-media-folder",tag="admin",params(("item_id"=uuid::Uuid,Path)),responses((status=200,body=MigrateFallbackResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn migrate_fallback_item(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<MigrateFallbackResponse>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

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

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct ProviderOrderPayload {
    pub provider_order: Vec<String>,
}

/// A library's saved custom provider order for one item type
/// (`"movie"`/`"episode"`) — the Admin Panel's drag-and-drop reordering UI.
/// An empty `provider_order` (never saved, or explicitly cleared) means
/// "use the admin's global priority order" — see `apply_provider_order`.
#[utoipa::path(get,path="/api/admin/libraries/{library_id}/provider-order/{item_type}",tag="admin",params(("library_id"=uuid::Uuid,Path),("item_type"=String,Path)),responses((status=200,body=ProviderOrderPayload),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn get_provider_order(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path((library_id, item_type)): Path<(Uuid, String)>,
) -> Result<Json<ProviderOrderPayload>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

    let order = lisek_db::get_library_provider_order(&pool, library_id, &item_type)
        .await
        .map_err(map_db_error)?
        .unwrap_or_default();
    Ok(Json(ProviderOrderPayload {
        provider_order: order,
    }))
}

#[utoipa::path(put,path="/api/admin/libraries/{library_id}/provider-order/{item_type}",tag="admin",params(("library_id"=Uuid,Path),("item_type"=String,Path,description="movie | episode")),request_body=ProviderOrderPayload,responses((status=200,body=Value),(status=400),(status=404),(status=401),(status=403),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn set_provider_order(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path((library_id, item_type)): Path<(Uuid, String)>,
    Json(req): Json<ProviderOrderPayload>,
) -> Result<Json<Value>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;

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

/// Faza 21: the metadata language order. `languages` is what the
/// administrator saved (empty: never set); `effective` is the order the
/// scanner uses (the saved one, or the server's default language, English,
/// the original language).
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct MetadataLanguagesResponse {
    pub languages: Vec<String>,
    pub effective: Vec<String>,
}

/// A new metadata language order: codes (`pl`, `en`, `pt-br` ...) plus
/// `original` (the production's own language) and `any`. Empty: back to
/// the default order.
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct MetadataLanguagesPayload {
    pub languages: Vec<String>,
}

async fn metadata_languages_response(
    pool: &lisek_db::PgPool,
) -> Result<MetadataLanguagesResponse, AdminError> {
    let languages = lisek_db::profiles::metadata_languages(pool)
        .await
        .map_err(map_db_error)?;
    let default = lisek_db::profiles::default_language(pool)
        .await
        .map_err(map_db_error)?;
    let effective = crate::metadata_cascade::effective_chain(&languages, &default);
    Ok(MetadataLanguagesResponse {
        languages,
        effective,
    })
}

#[utoipa::path(get,path="/api/admin/metadata/languages",tag="admin",responses((status=200,body=MetadataLanguagesResponse),(status=401),(status=403),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn get_metadata_languages(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<MetadataLanguagesResponse>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;
    Ok(Json(metadata_languages_response(&pool).await?))
}

#[utoipa::path(put,path="/api/admin/metadata/languages",tag="admin",request_body=MetadataLanguagesPayload,responses((status=200,body=MetadataLanguagesResponse),(status=400),(status=401),(status=403),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn set_metadata_languages(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Json(req): Json<MetadataLanguagesPayload>,
) -> Result<Json<MetadataLanguagesResponse>, AdminError> {
    let pool = state.pool().ok_or(AdminError::ServerNotReady)?;
    let languages = crate::metadata_cascade::validate(&req.languages)
        .map_err(|bad| AdminError::InvalidInput(format!("invalid metadata language: {bad}")))?;
    lisek_db::profiles::set_metadata_languages(&pool, &languages)
        .await
        .map_err(map_db_error)?;
    tracing::info!(?languages, admin = %auth.user.username, "metadata language order updated");
    Ok(Json(metadata_languages_response(&pool).await?))
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

pub(crate) fn map_db_error(err: lisek_db::DbError) -> AdminError {
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
