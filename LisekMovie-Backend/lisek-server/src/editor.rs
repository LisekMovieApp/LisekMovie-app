//! Faza 20: the metadata editor — films, whole series, single episodes —
//! and manual artwork (poster, backdrop, logo, episode still).
//!
//! The scope decides what a save means (see `lisek_db::editor`): a film
//! edits itself; an episode opened from a series page edits the series
//! (every episode, the series row renamed in place); an episode opened on
//! its own edits only its title, overview and air date. Changed fields are
//! locked automatically, so a rescan cannot take the title back from the
//! file name.
//!
//! Artwork arrives as the raw request body (an image) or as JSON
//! `{"url": "https://…"}`. It is decoded, scaled to a sane size and
//! re-encoded (JPEG; PNG for logos) before it touches the disk — whatever
//! was uploaded, only a clean picture is stored. It is written where the
//! image endpoints look (`store_image`: the media folder when the library
//! saves there, and the local cache), the kind is locked against provider
//! refreshes, and older media-folder files of that kind are ignored from
//! now on (`lisek_db::editor::artwork_source`).

use std::io::Cursor;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lisek_db::editor::{self as db, EditError, EpisodeContext};
use lisek_metadata::image_cache::{backdrop_cache_path, logo_cache_path, poster_cache_path, store_image};
use lisek_metadata::layout;

use crate::auth::AdminUser;
use crate::state::SharedState;

/// Largest artwork upload or download.
pub const MAX_ARTWORK_BYTES: usize = 25 * 1024 * 1024;
const MAX_SOURCE_SIDE: u32 = 16_000;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

pub struct EditorError(StatusCode, &'static str);
impl IntoResponse for EditorError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error_code": self.1 }))).into_response()
    }
}
impl From<lisek_db::DbError> for EditorError {
    fn from(e: lisek_db::DbError) -> Self {
        tracing::error!(error = %e, "metadata editor database error");
        internal()
    }
}
impl From<EditError> for EditorError {
    fn from(e: EditError) -> Self {
        match e {
            EditError::NotFound => not_found(),
            EditError::NotAnEpisode => bad("not_an_episode"),
            EditError::TitleTaken => EditorError(StatusCode::CONFLICT, "series_title_taken"),
            EditError::Database(e) => e.into(),
        }
    }
}
fn internal() -> EditorError {
    EditorError(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}
fn not_found() -> EditorError {
    EditorError(StatusCode::NOT_FOUND, "item_not_found")
}
fn bad(code: &'static str) -> EditorError {
    EditorError(StatusCode::BAD_REQUEST, code)
}
fn pool(state: &SharedState) -> Result<lisek_db::PgPool, EditorError> {
    state.pool().ok_or(EditorError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))
}

/// One item as the editor loads it: the item's own fields, its series and
/// episode side, and a version for the artwork previews.
#[derive(Serialize, utoipa::ToSchema)]
pub struct EditorItem {
    #[serde(flatten)]
    pub item: lisek_db::MediaItemDetail,
    #[serde(flatten)]
    pub episode: EpisodeContext,
    /// Changes whenever the item is saved: a cache-buster for previews.
    pub version: i64,
}

#[utoipa::path(get,path="/api/admin/media/{item_id}",tag="admin",params(("item_id"=Uuid,Path)),responses((status=200,body=EditorItem),(status=401),(status=403),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn get_item(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<EditorItem>, EditorError> {
    let pool = pool(&state)?;
    let item = lisek_db::get_media_item_detail(&pool, item_id).await?.ok_or_else(not_found)?;
    let episode = db::episode_context(&pool, item_id).await?;
    let version = db::item_version(&pool, item_id).await?.unwrap_or(0);
    Ok(Json(EditorItem { item, episode, version }))
}

/// What a save edits.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditScope {
    /// A film's own fields; for an episode the same as `episode`.
    #[default]
    Item,
    /// Every episode of the item's series.
    Series,
    /// Only this episode's own title, overview and air date.
    Episode,
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateMediaItemRequest {
    #[serde(default)]
    pub scope: EditScope,
    /// Film or series title (required for `item` on a film, and `series`).
    pub title: Option<String>,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    #[serde(default)]
    pub metadata_locked: bool,
    pub forced_provider: Option<String>,
    /// Fields protected from rescans and provider refreshes (`title`,
    /// `original_title`, `year`, `overview`, `poster`, `backdrop`, `logo`;
    /// for an episode `episode_title`, `episode_overview`, `thumb`). Fields
    /// this save changes are added automatically.
    #[serde(default)]
    pub locked_fields: Vec<String>,
    /// The administrator's own age rating (0–21); null: the provider's.
    #[serde(default)]
    pub age_rating: Option<i32>,
    pub episode_title: Option<String>,
    pub episode_overview: Option<String>,
    /// `YYYY-MM-DD`.
    pub episode_air_date: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UpdateMediaItemResponse {
    pub ok: bool,
    /// Items written (a series counts every episode).
    pub updated: u64,
    /// Fields locked automatically because this save changed them.
    pub auto_locked: Vec<String>,
}

fn clean(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn valid_air_date(value: &str) -> bool {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

/// Lock names of the shared fields that differ between before and after.
fn changed_shared(
    before: (&str, Option<&str>, Option<i32>, Option<&str>),
    after: (&str, Option<&str>, Option<i32>, Option<&str>),
) -> Vec<String> {
    let mut changed = Vec::new();
    if before.0 != after.0 {
        changed.push("title".to_string());
    }
    if before.1 != after.1 {
        changed.push("original_title".to_string());
    }
    if before.2 != after.2 {
        changed.push("year".to_string());
    }
    if before.3 != after.3 {
        changed.push("overview".to_string());
    }
    changed
}

/// The metadata editor's save. Never gated on `locked_fields` — an
/// administrator's own edit always applies; the locks only protect it
/// from automatic rescans and refreshes afterwards.
#[utoipa::path(put,path="/api/admin/media/{item_id}",tag="admin",params(("item_id"=Uuid,Path)),request_body=UpdateMediaItemRequest,responses((status=200,body=UpdateMediaItemResponse),(status=400),(status=401),(status=403),(status=404),(status=409,description="series_title_taken"),(status=503)),security(("lisek_auth"=[])))]
pub async fn update_item(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<UpdateMediaItemRequest>,
) -> Result<Json<UpdateMediaItemResponse>, EditorError> {
    let pool = pool(&state)?;
    let detail = lisek_db::get_media_item_detail(&pool, item_id).await?.ok_or_else(not_found)?;
    let is_episode = detail.item_type == "episode";
    let scope = match (req.scope, is_episode) {
        (EditScope::Item, true) => EditScope::Episode,
        (EditScope::Series | EditScope::Episode, false) => return Err(bad("not_an_episode")),
        (scope, _) => scope,
    };

    if scope == EditScope::Episode {
        let title = clean(req.episode_title.as_deref());
        let overview = clean(req.episode_overview.as_deref());
        let air_date = clean(req.episode_air_date.as_deref());
        if air_date.is_some_and(|d| !valid_air_date(d)) {
            return Err(bad("invalid_air_date"));
        }
        if title.is_some_and(|t| t.chars().count() > 500) || overview.is_some_and(|o| o.len() > 20_000) {
            return Err(bad("invalid_input"));
        }
        let before = db::episode_context(&pool, item_id).await?;
        let mut auto_locked = Vec::new();
        if clean(before.episode_title.as_deref()) != title {
            auto_locked.push("episode_title".to_string());
        }
        if clean(before.episode_overview.as_deref()) != overview {
            auto_locked.push("episode_overview".to_string());
        }
        let mut locks = req.locked_fields.clone();
        locks.extend(auto_locked.iter().cloned());
        db::update_episode(
            &pool,
            item_id,
            &db::EpisodeFields { title, overview, air_date, locked_fields: &locks },
        )
        .await?;
        tracing::info!(%item_id, admin = %auth.user.username, ?auto_locked, "episode metadata updated");
        return Ok(Json(UpdateMediaItemResponse { ok: true, updated: 1, auto_locked }));
    }

    let title = req.title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() || title.chars().count() > 500 {
        return Err(bad("title_empty"));
    }
    if req.year.is_some_and(|y| !(1800..=2200).contains(&y)) {
        return Err(bad("invalid_year"));
    }
    if req.age_rating.is_some_and(|age| !(0..=lisek_db::access::MAX_AGE_RATING).contains(&age)) {
        return Err(bad("invalid_age_rating"));
    }
    let original_title = clean(req.original_title.as_deref());
    let overview = clean(req.overview.as_deref());
    let before_title = if scope == EditScope::Series {
        db::episode_context(&pool, item_id)
            .await?
            .series_title
            .unwrap_or_else(|| detail.title.clone())
    } else {
        detail.title.clone()
    };
    let auto_locked = changed_shared(
        (&before_title, clean(detail.original_title.as_deref()), detail.year, clean(detail.overview.as_deref())),
        (title, original_title, req.year, overview),
    );
    let mut locks = req.locked_fields.clone();
    locks.extend(auto_locked.iter().cloned());
    let fields = db::SharedFields {
        title,
        original_title,
        year: req.year,
        overview,
        metadata_locked: req.metadata_locked,
        forced_provider: clean(req.forced_provider.as_deref()),
        locked_fields: &locks,
        age_rating: req.age_rating,
    };
    let updated = if scope == EditScope::Series {
        db::update_series(&pool, item_id, &fields).await?
    } else {
        db::update_item(&pool, item_id, &fields).await?;
        1
    };
    tracing::info!(%item_id, ?scope, updated, admin = %auth.user.username, ?auto_locked, "media metadata updated");
    Ok(Json(UpdateMediaItemResponse { ok: true, updated, auto_locked }))
}

// ---- Artwork -----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtworkKind {
    Poster,
    Backdrop,
    Logo,
    Thumb,
}

impl ArtworkKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "poster" => Some(Self::Poster),
            "backdrop" => Some(Self::Backdrop),
            "logo" => Some(Self::Logo),
            "thumb" => Some(Self::Thumb),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Poster => "poster",
            Self::Backdrop => "backdrop",
            Self::Logo => "logo",
            Self::Thumb => "thumb",
        }
    }
    /// Largest stored size; bigger pictures are scaled down to fit.
    fn bounds(self) -> (u32, u32) {
        match self {
            Self::Poster => (2000, 3000),
            Self::Backdrop | Self::Thumb => (3840, 2160),
            Self::Logo => (2000, 1000),
        }
    }
    fn media_filename(self) -> &'static str {
        match self {
            Self::Poster => "poster.jpg",
            Self::Backdrop => "backdrop.jpg",
            Self::Logo => "logo.png",
            Self::Thumb => "thumb.jpg",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ArtworkImageError {
    Unsupported,
    TooLarge,
}

/// Decodes any common picture, scales it into the kind's bounds and
/// re-encodes it: JPEG (q90) for photos, PNG for logos (transparency).
pub fn normalize_artwork(bytes: &[u8], kind: ArtworkKind) -> Result<Vec<u8>, ArtworkImageError> {
    use image::{ImageFormat, ImageReader, Limits};
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ArtworkImageError::Unsupported)?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP | ImageFormat::Gif)
    ) {
        return Err(ArtworkImageError::Unsupported);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let mut picture = reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => ArtworkImageError::TooLarge,
        _ => ArtworkImageError::Unsupported,
    })?;
    let (max_w, max_h) = kind.bounds();
    if picture.width() > max_w || picture.height() > max_h {
        picture = picture.resize(max_w, max_h, image::imageops::FilterType::Lanczos3);
    }
    let mut out = Vec::new();
    if kind == ArtworkKind::Logo {
        picture
            .to_rgba8()
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .map_err(|_| ArtworkImageError::Unsupported)?;
    } else {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&picture.to_rgb8())
            .map_err(|_| ArtworkImageError::Unsupported)?;
    }
    Ok(out)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ArtworkUrl {
    /// An `http(s)` link to the picture.
    pub url: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ArtworkResponse {
    pub kind: String,
    /// The image URL with a cache-buster.
    pub url: String,
    /// Items that got the new picture (a series counts every episode).
    pub items: usize,
}

/// Downloads a picture from an administrator-given link: http(s) only,
/// 30 s, at most `MAX_ARTWORK_BYTES`.
async fn download(url: &str) -> Result<Vec<u8>, EditorError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| bad("artwork_invalid_url"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(bad("artwork_invalid_url"));
    }
    let client = reqwest::Client::builder()
        .timeout(DOWNLOAD_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| internal())?;
    let failed = || EditorError(StatusCode::BAD_GATEWAY, "artwork_download_failed");
    let mut response = client.get(parsed).send().await.map_err(|e| {
        tracing::warn!(error = %e, "artwork download failed");
        failed()
    })?;
    if !response.status().is_success() {
        tracing::warn!(status = %response.status(), "artwork download refused");
        return Err(failed());
    }
    if response.content_length().is_some_and(|len| len > MAX_ARTWORK_BYTES as u64) {
        return Err(EditorError(StatusCode::PAYLOAD_TOO_LARGE, "artwork_too_large"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| failed())? {
        if bytes.len() + chunk.len() > MAX_ARTWORK_BYTES {
            return Err(EditorError(StatusCode::PAYLOAD_TOO_LARGE, "artwork_too_large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn cache_path(config: &FsPath, kind: ArtworkKind, id: Uuid) -> PathBuf {
    match kind {
        ArtworkKind::Poster => poster_cache_path(config, id),
        ArtworkKind::Backdrop => backdrop_cache_path(config, id),
        ArtworkKind::Logo => logo_cache_path(config, id),
        ArtworkKind::Thumb => layout::episode_still_path(config, &id.to_string()),
    }
}

/// Replaces one picture. Poster, backdrop and logo of an episode belong to
/// its series (every episode gets them); `thumb` is an episode's still.
#[utoipa::path(put,path="/api/admin/media/{item_id}/artwork/{kind}",tag="admin",params(("item_id"=Uuid,Path),("kind"=String,Path,description="poster, backdrop, logo or thumb")),request_body(content=ArtworkUrl,description="JSON {url}, or the raw picture (image/*)"),responses((status=200,body=ArtworkResponse),(status=400),(status=401),(status=403),(status=404),(status=413),(status=502,description="artwork_download_failed")),security(("lisek_auth"=[])))]
pub async fn set_artwork(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path((item_id, kind)): Path<(Uuid, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<ArtworkResponse>, EditorError> {
    let kind = ArtworkKind::parse(&kind).ok_or_else(|| bad("invalid_artwork_kind"))?;
    let pool = pool(&state)?;
    let detail = lisek_db::get_media_item_detail(&pool, item_id).await?.ok_or_else(not_found)?;
    let is_episode = detail.item_type == "episode";
    if kind == ArtworkKind::Thumb && !is_episode {
        return Err(bad("not_an_episode"));
    }

    let raw = axum::body::to_bytes(body, MAX_ARTWORK_BYTES + 64 * 1024)
        .await
        .map_err(|_| EditorError(StatusCode::PAYLOAD_TOO_LARGE, "artwork_too_large"))?;
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let source = if is_json {
        let request: ArtworkUrl = serde_json::from_slice(&raw).map_err(|_| bad("artwork_invalid_url"))?;
        download(request.url.trim()).await?
    } else {
        if raw.len() > MAX_ARTWORK_BYTES {
            return Err(EditorError(StatusCode::PAYLOAD_TOO_LARGE, "artwork_too_large"));
        }
        raw.to_vec()
    };
    if source.is_empty() {
        return Err(bad("artwork_empty"));
    }
    let picture = tokio::task::spawn_blocking(move || normalize_artwork(&source, kind))
        .await
        .map_err(|_| internal())?
        .map_err(|e| match e {
            ArtworkImageError::Unsupported => bad("artwork_invalid_image"),
            ArtworkImageError::TooLarge => EditorError(StatusCode::PAYLOAD_TOO_LARGE, "artwork_too_large"),
        })?;

    let series_wide = is_episode && kind != ArtworkKind::Thumb;
    let ids = if series_wide {
        let ids = lisek_db::episodes::series_episode_ids(&pool, item_id).await?;
        if ids.is_empty() { vec![item_id] } else { ids }
    } else {
        vec![item_id]
    };
    let file_path = lisek_db::get_media_item_file_path(&pool, item_id).await?.ok_or_else(not_found)?;
    let save_in_folder = lisek_db::get_library(&pool, detail.library_id)
        .await?
        .is_some_and(|l| l.save_metadata_in_media_folder);
    let file = FsPath::new(&file_path);
    let media_dest = match (kind, save_in_folder) {
        (ArtworkKind::Thumb, _) | (_, false) => None,
        (_, true) if series_wide => layout::series_folder(file).map(|d| d.join(kind.media_filename())),
        (_, true) => file.parent().map(|d| d.join(kind.media_filename())),
    };

    // Recorded before the files are written: a media-folder file older than
    // this moment no longer counts, the one written below does.
    let at = chrono::Utc::now().timestamp_millis();
    db::record_artwork_override(&pool, &ids, kind.name(), at).await?;
    let config = crate::state::config_dir();
    let mut written = 0usize;
    for (n, id) in ids.iter().enumerate() {
        let media = if n == 0 { media_dest.as_deref() } else { None };
        match store_image(&picture, media, &cache_path(&config, kind, *id)).await {
            Ok(_) => written += 1,
            Err(e) => {
                tracing::error!(%id, error = %e, kind = kind.name(), "saving artwork failed");
                return Err(EditorError(StatusCode::INTERNAL_SERVER_ERROR, "artwork_save_failed"));
            }
        }
    }
    tracing::info!(%item_id, kind = kind.name(), items = written, admin = %auth.user.username, "artwork replaced");
    let url = match kind {
        ArtworkKind::Thumb => format!("/api/media/{item_id}/thumb?v={at}"),
        _ => format!("/api/media/{item_id}/{}?v={at}", kind.name()),
    };
    Ok(Json(ArtworkResponse { kind: kind.name().to_string(), url, items: written }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, image::Rgba(rgba)))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn posters_are_scaled_into_bounds_as_jpeg() {
        let big = png(4000, 6000, [10, 200, 10, 255]);
        let out = normalize_artwork(&big, ArtworkKind::Poster).unwrap();
        let back = image::load_from_memory_with_format(&out, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (2000, 3000));
    }

    #[test]
    fn small_pictures_keep_their_size_and_logos_stay_png() {
        let logo = png(300, 100, [255, 255, 255, 0]);
        let out = normalize_artwork(&logo, ArtworkKind::Logo).unwrap();
        let back = image::load_from_memory_with_format(&out, image::ImageFormat::Png).unwrap();
        assert_eq!((back.width(), back.height()), (300, 100));
        assert_eq!(back.to_rgba8().get_pixel(5, 5)[3], 0, "transparency kept");
    }

    #[test]
    fn non_pictures_are_refused() {
        assert_eq!(normalize_artwork(b"<html>", ArtworkKind::Poster), Err(ArtworkImageError::Unsupported));
        assert_eq!(
            normalize_artwork(b"<svg xmlns='http://www.w3.org/2000/svg'/>", ArtworkKind::Backdrop),
            Err(ArtworkImageError::Unsupported)
        );
    }

    #[test]
    fn kinds_and_air_dates() {
        assert_eq!(ArtworkKind::parse("logo"), Some(ArtworkKind::Logo));
        assert_eq!(ArtworkKind::parse("../poster"), None);
        assert!(valid_air_date("2026-11-01"));
        assert!(!valid_air_date("1.11.2026"));
    }

    #[test]
    fn only_changed_fields_lock_themselves() {
        let changed = changed_shared(
            ("Old", None, Some(2000), Some("x")),
            ("New", None, Some(2000), Some("y")),
        );
        assert_eq!(changed, vec!["title", "overview"]);
    }
}
