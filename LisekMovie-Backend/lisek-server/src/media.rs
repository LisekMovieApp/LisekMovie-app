//! Client-facing media endpoints: the home-page feed (hero item + four
//! rows) and serving one item's poster/backdrop image. No `require_admin`
//! gate here — unlike everything in `admin.rs`, browsing the library and
//! viewing its images is for every signed-in account, not just admins;
//! `AuthUser` alone (see its own module's doc comment on why every
//! data-serving route needs it) is this module's whole security boundary.
//!
//! As of Faza 5 round 2, an item's image doesn't always live in the local
//! `config_dir` cache — a library with `save_metadata_in_media_folder`
//! turned on has it written directly next to the video file instead (see
//! `metadata_enrichment::enrich_item`). [`serve_item_image`] tries that
//! media-folder copy first (looking the item's `file_path` up fresh on
//! every request, rather than trusting a value cached at scan time — an
//! admin can toggle the setting or move files between scans) and only
//! falls back to the local cache when there's no media-folder copy to
//! read, which also transparently covers a library that has the setting
//! turned off, or a fallback-flagged item whose image only ever made it
//! into the cache in the first place.

use std::path::{Path as FsPath, PathBuf};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::state::SharedState;

/// One item as the frontend consumes it: `lisek_core::MediaItem` plus the
/// two image URLs, which aren't stored anywhere — they're just this item's
/// id slotted into the two routes below. Always present, even for an item
/// with no cached image yet; the routes themselves 404 rather than this
/// struct trying to know in advance whether a file exists on disk.
#[derive(Serialize)]
pub struct MediaItemResponse {
    pub id: Uuid,
    pub item_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub poster_url: String,
    pub backdrop_url: String,
}

impl From<lisek_core::MediaItem> for MediaItemResponse {
    fn from(item: lisek_core::MediaItem) -> Self {
        Self {
            poster_url: format!("/api/media/{}/poster", item.id),
            backdrop_url: format!("/api/media/{}/backdrop", item.id),
            id: item.id,
            item_type: item.item_type,
            title: item.title,
            year: item.year,
            overview: item.overview,
            season: item.season,
            episode: item.episode,
        }
    }
}

/// How many cards each row fetches — generous enough to fill and slightly
/// overflow a typical carousel viewport (`MediaRow`'s own Faza 4 skeleton
/// count defaulted to 10) without pulling a whole library's worth of items
/// into one response. Top 10 is, structurally, always exactly ten.
const ROW_LIMIT: i64 = 14;
const TOP10_LIMIT: i64 = 10;

#[derive(Serialize)]
pub struct HomeFeedResponse {
    pub hero: Option<MediaItemResponse>,
    pub recently_added: Vec<MediaItemResponse>,
    pub top10: Vec<MediaItemResponse>,
    pub recommended: Vec<MediaItemResponse>,
    pub popular: Vec<MediaItemResponse>,
}

/// Everything the home page needs in one round trip — see `lisek-db`'s
/// `get_hero_item`/`list_recently_added`/`list_top_today`/
/// `list_recommended`/`list_popular` doc comments for exactly what each row
/// means and, for `top10`/`recommended`/`popular`, the honest placeholder
/// ranking each uses in the absence of any real watch-history/popularity
/// tracking (not yet built — see the roadmap).
pub async fn home_feed(
    _auth: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<HomeFeedResponse>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;

    let hero = lisek_db::get_hero_item(&pool)
        .await
        .map_err(|_| MediaError::Internal)?;
    let recently_added = lisek_db::list_recently_added(&pool, ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    let top10 = lisek_db::list_top_today(&pool, TOP10_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    let recommended = lisek_db::list_recommended(&pool, ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    let popular = lisek_db::list_popular(&pool, ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;

    Ok(Json(HomeFeedResponse {
        hero: hero.map(Into::into),
        recently_added: recently_added.into_iter().map(Into::into).collect(),
        top10: top10.into_iter().map(Into::into).collect(),
        recommended: recommended.into_iter().map(Into::into).collect(),
        popular: popular.into_iter().map(Into::into).collect(),
    }))
}

/// Serves one item's poster. `AuthUser` accepts a `?token=` query param
/// specifically so a plain `<img src>` can hit this route — see
/// `auth.rs::token_from_query`'s doc comment.
pub async fn poster_image(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Response {
    serve_item_image(
        &state,
        item_id,
        "poster.jpg",
        lisek_metadata::image_cache::poster_cache_path,
    )
    .await
}

/// Same as [`poster_image`] for the backdrop image.
pub async fn backdrop_image(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Response {
    serve_item_image(
        &state,
        item_id,
        "backdrop.jpg",
        lisek_metadata::image_cache::backdrop_cache_path,
    )
    .await
}

/// Tries the media-folder copy (`{media_folder_filename}` next to the
/// item's own `file_path`, looked up fresh via `lisek_db::
/// get_media_item_file_path`) before the local `config_dir` cache
/// (`cache_path_fn`). Any failure along the media-folder path — no pool
/// yet, no such item, no file path recorded, no file actually sitting
/// there — is treated identically as "no media-folder candidate, try the
/// cache" rather than as the request's final error; only the cache read at
/// the end decides between success and 404, exactly like before this
/// media-folder lookup was added.
async fn serve_item_image(
    state: &SharedState,
    item_id: Uuid,
    media_folder_filename: &str,
    cache_path_fn: fn(&FsPath, Uuid) -> PathBuf,
) -> Response {
    if let Some(bytes) = read_media_folder_image(state, item_id, media_folder_filename).await {
        return image_response(bytes);
    }

    let cache_path = cache_path_fn(&crate::state::config_dir(), item_id);
    match tokio::fs::read(&cache_path).await {
        Ok(bytes) => image_response(bytes),
        // Neither a media-folder copy nor a cached one — either this item
        // has no provider match yet, or no provider is configured at all.
        // A 404 is exactly right here: the frontend's `<img>` `onerror`
        // handler swaps in a placeholder, same as any other missing image.
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn read_media_folder_image(
    state: &SharedState,
    item_id: Uuid,
    filename: &str,
) -> Option<Vec<u8>> {
    let pool = state.pool().await?;
    let file_path = lisek_db::get_media_item_file_path(&pool, item_id)
        .await
        .ok()??;
    let media_path = FsPath::new(&file_path).parent()?.join(filename);
    tokio::fs::read(&media_path).await.ok()
}

fn image_response(bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            // Not content-addressed the way the old cache-only path was
            // (a media-folder file can be replaced by the admin at any
            // time outside a scan), but still only ever changes on a
            // fresh scan+enrichment cycle in the normal case — an hour of
            // browser caching remains a reasonable trade.
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        bytes,
    )
        .into_response()
}

#[derive(Debug)]
pub enum MediaError {
    ServerNotReady,
    Internal,
}

impl IntoResponse for MediaError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            MediaError::ServerNotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "server_not_ready",
                "The server isn't fully configured yet.",
            ),
            MediaError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Something went wrong while loading the home feed.",
            ),
        };

        (
            status,
            Json(json!({ "error_code": error_code, "message": message })),
        )
            .into_response()
    }
}
