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

use std::collections::BTreeMap;
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
    /// Faza 22: the ids the item is matched with, by kind (`tmdb` →
    /// `"603"`, `imdb` → `"tt0133093"`).
    pub provider_ids: BTreeMap<String, String>,
    /// The plugin of the last match.
    pub provider: Option<String>,
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
    let identity = lisek_db::identity::stored_identity(&pool, item_id).await?;
    Ok(Json(EditorItem { item, episode, version, provider_ids: identity.ids, provider: identity.provider }))
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
    /// Faza 21: 0–10 (detail page stars); null clears it; omitted: unchanged.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<f64>)]
    pub rating: Option<Option<f64>>,
    /// Faza 21: directors shown instead of the provider's credits; empty:
    /// the provider's; omitted: unchanged.
    #[serde(default)]
    pub directors: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<f64>)]
    pub episode_rating: Option<Option<f64>>,
    #[serde(default)]
    pub episode_directors: Option<Vec<String>>,
    pub episode_title: Option<String>,
    pub episode_overview: Option<String>,
    /// `YYYY-MM-DD`.
    pub episode_air_date: Option<String>,
    /// Faza 22: every id the item should have, by kind (`tmdb`, `tvdb`,
    /// `imdb`, `anidb`, `anilist`, `mal`, `omdb`, `imvdb`); a kind left out
    /// is removed. For an episode the ids of its series. Omitted: unchanged.
    /// Changed ids are locked and the metadata is fetched again by them.
    #[serde(default)]
    pub provider_ids: Option<BTreeMap<String, String>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct UpdateMediaItemResponse {
    pub ok: bool,
    /// Items written (a series counts every episode).
    pub updated: u64,
    /// Fields locked automatically because this save changed them.
    pub auto_locked: Vec<String>,
    /// Faza 22: the ids changed and the metadata was fetched again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refetched: Option<crate::refetch::RefetchSummary>,
}

fn clean(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

/// A field that is there (even as `null`) is `Some`; a missing one `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Trimmed, non-empty names; `None` when the list is unusable.
fn clean_names(names: &[String]) -> Option<Vec<String>> {
    let names: Vec<String> = names.iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
    (names.len() <= 20 && names.iter().all(|n| n.chars().count() <= 200)).then_some(names)
}

fn valid_rating(rating: Option<f64>) -> bool {
    rating.is_none_or(|r| r.is_finite() && (0.0..=10.0).contains(&r))
}

/// A rating as stored: one decimal.
fn round_rating(rating: Option<f64>) -> Option<f64> {
    rating.map(|r| (r * 10.0).round() / 10.0)
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

/// Faza 22: ids as typed, validated and trimmed (`None`: a bad one).
fn clean_ids(ids: &BTreeMap<String, String>) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (kind, id) in ids {
        let kind = kind.trim().to_ascii_lowercase();
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        let id = if kind == "imdb" || kind == "omdb" { id.to_ascii_lowercase() } else { id.to_owned() };
        if !lisek_db::identity::ID_KINDS.contains(&kind.as_str()) || !lisek_db::identity::valid_id(&kind, &id) {
            return None;
        }
        out.insert(kind, id);
    }
    Some(out)
}

/// Faza 22: after a save — writes changed ids to the item (a series: every
/// episode), locks them and fetches the metadata again by them; or, when
/// only other fields changed, locks the ids so a scan cannot re-identify
/// the item. Returns the refetch summary when the ids changed.
async fn save_identity(
    state: &SharedState,
    pool: &lisek_db::PgPool,
    item_id: Uuid,
    wanted: Option<&BTreeMap<String, String>>,
    before: &BTreeMap<String, String>,
    anything_changed: bool,
) -> Result<Option<crate::refetch::RefetchSummary>, EditorError> {
    let scope = lisek_db::identity::identity_scope(pool, item_id).await?;
    match wanted {
        Some(ids) if ids != before => {
            lisek_db::identity::set_provider_ids(pool, &scope, ids).await?;
            tracing::info!(%item_id, ?before, after = ?ids, items = scope.len(), "provider ids changed; fetching metadata again");
            let summary = crate::refetch::refetch(state, pool, &scope).await?;
            Ok(Some(summary))
        }
        _ => {
            if anything_changed && !before.is_empty() {
                lisek_db::identity::lock_provider_ids(pool, &scope).await?;
            }
            Ok(None)
        }
    }
}

/// Fetches an item's metadata again now (a series: every episode), by its
/// stored ids when it has them.
#[utoipa::path(post,path="/api/admin/media/{item_id}/refresh",tag="admin",params(("item_id"=Uuid,Path)),responses((status=200,body=crate::refetch::RefetchSummary),(status=401),(status=403),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn refresh_item(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<crate::refetch::RefetchSummary>, EditorError> {
    let pool = pool(&state)?;
    lisek_db::get_media_item_detail(&pool, item_id).await?.ok_or_else(not_found)?;
    let scope = lisek_db::identity::identity_scope(&pool, item_id).await?;
    let summary = crate::refetch::refetch(&state, &pool, &scope).await?;
    tracing::info!(%item_id, ?summary, admin = %auth.user.username, "metadata fetched again");
    Ok(Json(summary))
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
    let wanted_ids = match &req.provider_ids {
        Some(ids) => Some(clean_ids(ids).ok_or_else(|| bad("invalid_provider_id"))?),
        None => None,
    };
    let ids_before = lisek_db::identity::stored_identity(&pool, item_id).await?.ids;
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
        let rating = match req.episode_rating {
            Some(rating) if !valid_rating(rating) => return Err(bad("invalid_rating")),
            Some(rating) => round_rating(rating),
            None => before.episode_rating,
        };
        let directors = match &req.episode_directors {
            Some(names) => clean_names(names).ok_or_else(|| bad("invalid_input"))?,
            None => before.episode_directors.clone(),
        };
        let mut auto_locked = Vec::new();
        if clean(before.episode_title.as_deref()) != title {
            auto_locked.push("episode_title".to_string());
        }
        if clean(before.episode_overview.as_deref()) != overview {
            auto_locked.push("episode_overview".to_string());
        }
        if before.episode_rating != rating {
            auto_locked.push("episode_rating".to_string());
        }
        if before.episode_directors != directors {
            auto_locked.push("episode_directors".to_string());
        }
        let mut locks = req.locked_fields.clone();
        locks.extend(auto_locked.iter().cloned());
        db::update_episode(
            &pool,
            item_id,
            &db::EpisodeFields { title, overview, air_date, rating, directors: &directors, locked_fields: &locks },
        )
        .await?;
        tracing::info!(%item_id, admin = %auth.user.username, ?auto_locked, "episode metadata updated");
        let refetched =
            save_identity(&state, &pool, item_id, wanted_ids.as_ref(), &ids_before, !auto_locked.is_empty()).await?;
        return Ok(Json(UpdateMediaItemResponse { ok: true, updated: 1, auto_locked, refetched }));
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
    let context = db::episode_context(&pool, item_id).await?;
    let before_title = if scope == EditScope::Series {
        context.series_title.clone().unwrap_or_else(|| detail.title.clone())
    } else {
        detail.title.clone()
    };
    let rating = match req.rating {
        Some(rating) if !valid_rating(rating) => return Err(bad("invalid_rating")),
        Some(rating) => round_rating(rating),
        None => context.rating,
    };
    let directors = match &req.directors {
        Some(names) => clean_names(names).ok_or_else(|| bad("invalid_input"))?,
        None => context.directors.clone(),
    };
    let mut auto_locked = changed_shared(
        (&before_title, clean(detail.original_title.as_deref()), detail.year, clean(detail.overview.as_deref())),
        (title, original_title, req.year, overview),
    );
    if context.rating != rating {
        auto_locked.push("rating".to_string());
    }
    if context.directors != directors {
        auto_locked.push("directors".to_string());
    }
    // Faza 22: any change locks the ids the item is matched with.
    let ids_changed = wanted_ids.as_ref().is_some_and(|ids| *ids != ids_before);
    if (!auto_locked.is_empty() || ids_changed) && (!ids_before.is_empty() || ids_changed) {
        auto_locked.push(lisek_db::identity::PROVIDER_IDS_LOCK.to_string());
    }
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
        rating,
        directors: &directors,
    };
    let updated = if scope == EditScope::Series {
        db::update_series(&pool, item_id, &fields).await?
    } else {
        db::update_item(&pool, item_id, &fields).await?;
        1
    };
    tracing::info!(%item_id, ?scope, updated, admin = %auth.user.username, ?auto_locked, "media metadata updated");
    let refetched = save_identity(&state, &pool, item_id, wanted_ids.as_ref(), &ids_before, false).await?;
    Ok(Json(UpdateMediaItemResponse { ok: true, updated, auto_locked, refetched }))
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

/// The picture of an artwork request: the raw body, or the image at the
/// link in a JSON body.
async fn read_picture(headers: &HeaderMap, body: Body) -> Result<Vec<u8>, EditorError> {
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
    Ok(source)
}

async fn clean_picture(source: Vec<u8>, kind: ArtworkKind) -> Result<Vec<u8>, EditorError> {
    tokio::task::spawn_blocking(move || normalize_artwork(&source, kind))
        .await
        .map_err(|_| internal())?
        .map_err(|e| match e {
            ArtworkImageError::Unsupported => bad("artwork_invalid_image"),
            ArtworkImageError::TooLarge => EditorError(StatusCode::PAYLOAD_TOO_LARGE, "artwork_too_large"),
        })
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

    let source = read_picture(&headers, body).await?;
    let picture = clean_picture(source, kind).await?;

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
    // Faza 22: a replaced picture also keeps the item's identity.
    lisek_db::identity::lock_provider_ids(&pool, &lisek_db::identity::identity_scope(&pool, item_id).await?).await?;
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

// ---- Seasons (Faza 21) -------------------------------------------------------

#[utoipa::path(get,path="/api/admin/media/{item_id}/seasons/{season}",tag="admin",params(("item_id"=Uuid,Path,description="Any episode of the series"),("season"=i32,Path)),responses((status=200,body=db::SeasonDetail),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn get_season(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path((item_id, season)): Path<(Uuid, i32)>,
) -> Result<Json<db::SeasonDetail>, EditorError> {
    let pool = pool(&state)?;
    Ok(Json(db::season_detail(&pool, item_id, season).await?.ok_or_else(not_found)?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateSeasonRequest {
    /// "Sezon 1", "Specials" …; null: the provider's (or none).
    pub name: Option<String>,
    pub overview: Option<String>,
    pub air_year: Option<i32>,
    /// `name`, `overview`, `air_year`, `poster`; changed fields are added.
    #[serde(default)]
    pub locked_fields: Vec<String>,
}

/// A season's name, overview and year.
#[utoipa::path(put,path="/api/admin/media/{item_id}/seasons/{season}",tag="admin",params(("item_id"=Uuid,Path),("season"=i32,Path)),request_body=UpdateSeasonRequest,responses((status=200,body=UpdateMediaItemResponse),(status=400),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn update_season(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path((item_id, season)): Path<(Uuid, i32)>,
    Json(req): Json<UpdateSeasonRequest>,
) -> Result<Json<UpdateMediaItemResponse>, EditorError> {
    let pool = pool(&state)?;
    let before = db::season_detail(&pool, item_id, season).await?.ok_or_else(not_found)?;
    let name = clean(req.name.as_deref());
    let overview = clean(req.overview.as_deref());
    if name.is_some_and(|n| n.chars().count() > 200) || overview.is_some_and(|o| o.len() > 20_000) {
        return Err(bad("invalid_input"));
    }
    if req.air_year.is_some_and(|y| !(1800..=2200).contains(&y)) {
        return Err(bad("invalid_year"));
    }
    let mut auto_locked = Vec::new();
    if clean(before.name.as_deref()) != name {
        auto_locked.push("name".to_string());
    }
    if clean(before.overview.as_deref()) != overview {
        auto_locked.push("overview".to_string());
    }
    if before.air_year != req.air_year {
        auto_locked.push("air_year".to_string());
    }
    let mut locks = req.locked_fields.clone();
    locks.extend(auto_locked.iter().cloned());
    db::update_season(
        &pool,
        before.series_id,
        season,
        &db::SeasonFields { name, overview, air_year: req.air_year, locked_fields: &locks },
    )
    .await?;
    if !auto_locked.is_empty() {
        lisek_db::identity::lock_provider_ids(&pool, &lisek_db::identity::identity_scope(&pool, item_id).await?).await?;
    }
    tracing::info!(series = %before.series_id, season, admin = %auth.user.username, ?auto_locked, "season metadata updated");
    Ok(Json(UpdateMediaItemResponse { ok: true, updated: 1, auto_locked, refetched: None }))
}

/// Replaces a season's poster (raw picture or JSON `{url}`).
#[utoipa::path(put,path="/api/admin/media/{item_id}/seasons/{season}/poster",tag="admin",params(("item_id"=Uuid,Path),("season"=i32,Path)),request_body(content=ArtworkUrl,description="JSON {url}, or the raw picture (image/*)"),responses((status=200,body=ArtworkResponse),(status=400),(status=401),(status=403),(status=404),(status=413),(status=502)),security(("lisek_auth"=[])))]
pub async fn set_season_poster(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path((item_id, season)): Path<(Uuid, i32)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<ArtworkResponse>, EditorError> {
    let pool = pool(&state)?;
    let detail = db::season_detail(&pool, item_id, season).await?.ok_or_else(not_found)?;
    let source = read_picture(&headers, body).await?;
    let picture = clean_picture(source, ArtworkKind::Poster).await?;
    let file_path = lisek_db::get_media_item_file_path(&pool, item_id).await?.ok_or_else(not_found)?;
    let library_id = lisek_db::get_media_item_detail(&pool, item_id).await?.ok_or_else(not_found)?.library_id;
    let save_in_folder = lisek_db::get_library(&pool, library_id).await?.is_some_and(|l| l.save_metadata_in_media_folder);
    let media_dest = save_in_folder
        .then(|| layout::series_folder(FsPath::new(&file_path)).map(|d| d.join(layout::season_poster_name(season))))
        .flatten();
    let cache = layout::season_poster_cache_path(&crate::state::config_dir(), &detail.series_id.to_string(), season);
    let at = chrono::Utc::now().timestamp_millis();
    db::record_season_poster(&pool, detail.series_id, season, at).await?;
    lisek_db::identity::lock_provider_ids(&pool, &lisek_db::identity::identity_scope(&pool, item_id).await?).await?;
    store_image(&picture, media_dest.as_deref(), &cache).await.map_err(|e| {
        tracing::error!(error = %e, season, "saving a season poster failed");
        EditorError(StatusCode::INTERNAL_SERVER_ERROR, "artwork_save_failed")
    })?;
    tracing::info!(series = %detail.series_id, season, admin = %auth.user.username, "season poster replaced");
    Ok(Json(ArtworkResponse {
        kind: "season_poster".into(),
        url: format!("/api/media/{item_id}/seasons/{season}/poster?v={at}"),
        items: 1,
    }))
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
    fn a_null_rating_clears_and_a_missing_one_keeps() {
        let clear: UpdateMediaItemRequest = serde_json::from_str(r#"{"rating": null, "episode_rating": 7.5}"#).unwrap();
        assert_eq!(clear.rating, Some(None));
        assert_eq!(clear.episode_rating, Some(Some(7.5)));
        let keep: UpdateMediaItemRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(keep.rating, None);
        assert!(valid_rating(Some(10.0)) && !valid_rating(Some(10.5)) && !valid_rating(Some(f64::NAN)));
        assert_eq!(round_rating(Some(7.26)), Some(7.3));
        assert_eq!(clean_names(&[" Anna ".into(), "".into()]), Some(vec!["Anna".to_string()]));
        assert_eq!(clean_names(&vec!["x".to_string(); 21]), None);
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
