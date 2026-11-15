//! Faza 26: own TV channels — the administrator's side.
//!
//! - `/api/admin/tv/channels...`: channels, their schedule (slots), logo,
//!   the timeline preview and a manual rebuild;
//! - `/api/admin/tv/fillers/...`: ad/bumper folders on the server, their
//!   files (probed with ffprobe) and the presets that say how they are
//!   inserted;
//! - [`run_builder`]: the background service that keeps every channel's
//!   timeline planned 72 hours ahead (every 10 minutes, and at once after
//!   an edit).
//!
//! The schedule rules live in `lisek-tv`, the storage and the atomic
//! rebuild in `lisek_db::tv`; this module validates input and glues them.

use std::io::Cursor;
use std::path::{Path as FsPath, PathBuf};
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lisek_db::tv::{self as db, ChannelInput, FillerPresetRow, RebuildMode, SlotInput};

use crate::auth::AdminUser;
use crate::state::SharedState;

const BUILD_INTERVAL: Duration = Duration::from_secs(10 * 60);
const MAX_SLOTS: usize = 200;
const MAX_LOGO_BYTES: usize = 5 * 1024 * 1024;
const MAX_FILLER_FILES: usize = 5000;
const FILLER_EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "webm", "m4v", "mpg", "mpeg", "ts", "wmv", "flv"];
const QUALITIES: &[&str] = &["480p", "720p", "1080p"];

#[derive(Debug)]
pub struct TvError {
    status: StatusCode,
    code: &'static str,
    detail: Option<String>,
}

impl TvError {
    pub fn new(status: StatusCode, code: &'static str) -> Self {
        TvError { status, code, detail: None }
    }

    fn invalid(code: &'static str, detail: impl Into<String>) -> Self {
        TvError { status: StatusCode::BAD_REQUEST, code, detail: Some(detail.into()) }
    }
}

impl IntoResponse for TvError {
    fn into_response(self) -> Response {
        (self.status, Json(serde_json::json!({ "error_code": self.code, "detail": self.detail }))).into_response()
    }
}

impl From<lisek_db::DbError> for TvError {
    fn from(e: lisek_db::DbError) -> Self {
        tracing::error!(error = %e, "TV database error");
        TvError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
    }
}

pub fn pool(state: &SharedState) -> Result<lisek_db::PgPool, TvError> {
    state.pool().ok_or(TvError::new(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))
}

fn not_found() -> TvError {
    TvError::new(StatusCode::NOT_FOUND, "channel_not_found")
}

// ---- Validation ----------------------------------------------------------------

/// A channel number: digits, optionally with one or two decimals ("5", "5.1").
pub fn valid_number(number: &str) -> bool {
    let (whole, fraction) = number.split_once('.').map_or((number, None), |(w, f)| (w, Some(f)));
    (1..=5).contains(&whole.len())
        && whole.bytes().all(|b| b.is_ascii_digit())
        && fraction.is_none_or(|f| (1..=2).contains(&f.len()) && f.bytes().all(|b| b.is_ascii_digit()))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ChannelBody {
    /// Empty: the smallest free number.
    #[serde(default)]
    pub number: String,
    pub name: String,
    #[serde(default)]
    pub group_title: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub timezone: String,
    #[serde(default = "default_quality")]
    pub quality: String,
    #[serde(default)]
    pub fallback_filler_id: Option<Uuid>,
    #[serde(default)]
    pub keep_running: bool,
}

fn yes() -> bool {
    true
}
fn default_quality() -> String {
    "720p".into()
}

/// A channel body checked and trimmed. Number uniqueness is checked by the caller.
pub fn clean_channel(body: ChannelBody) -> Result<ChannelInput, TvError> {
    let name = body.name.trim().to_string();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(TvError::invalid("invalid_name", "1-100 characters"));
    }
    let number = body.number.trim().to_string();
    if !number.is_empty() && !valid_number(&number) {
        return Err(TvError::invalid("invalid_number", number));
    }
    if lisek_tv::parse_zone(body.timezone.trim()).is_none() {
        return Err(TvError::invalid("invalid_timezone", body.timezone));
    }
    if !QUALITIES.contains(&body.quality.as_str()) {
        return Err(TvError::invalid("invalid_quality", body.quality));
    }
    let group_title = body.group_title.map(|g| g.trim().to_string()).filter(|g| !g.is_empty());
    if group_title.as_ref().is_some_and(|g| g.chars().count() > 100) {
        return Err(TvError::invalid("invalid_group", "at most 100 characters"));
    }
    Ok(ChannelInput {
        number,
        name,
        group_title,
        enabled: body.enabled,
        timezone: body.timezone.trim().to_string(),
        quality: body.quality,
        fallback_filler_id: body.fallback_filler_id,
        keep_running: body.keep_running,
    })
}

/// A schedule checked: every field in range, numbered for the message.
pub fn clean_slots(slots: Vec<SlotInput>) -> Result<Vec<SlotInput>, TvError> {
    if slots.len() > MAX_SLOTS {
        return Err(TvError::invalid("too_many_slots", MAX_SLOTS.to_string()));
    }
    let mut out = Vec::with_capacity(slots.len());
    for (i, mut s) in slots.into_iter().enumerate() {
        let bad = |what: &str| TvError::invalid("invalid_slot", format!("{}: {what}", i + 1));
        if let Some(start) = s.start_time.as_deref()
            && chrono::NaiveTime::parse_from_str(start, "%H:%M").is_err()
        {
            return Err(bad("start_time"));
        }
        if !matches!(s.start_mode.as_str(), "strict" | "flexible") {
            return Err(bad("start_mode"));
        }
        if !(1..=127).contains(&s.days_of_week) {
            return Err(bad("days_of_week"));
        }
        if !matches!(s.source_kind.as_str(), "library" | "series" | "movie" | "collection") {
            return Err(bad("source_kind"));
        }
        if lisek_tv::PlayOrder::from_db(&s.play_order).is_none() {
            return Err(bad("play_order"));
        }
        match s.mode.as_str() {
            "one" | "flood" => {
                s.count = None;
                s.duration_minutes = None;
            }
            "multiple" => {
                if !s.count.is_some_and(|c| (1..=100).contains(&c)) {
                    return Err(bad("count"));
                }
                s.duration_minutes = None;
            }
            "duration" => {
                if !s.duration_minutes.is_some_and(|m| (1..=1440).contains(&m)) {
                    return Err(bad("duration_minutes"));
                }
                s.count = None;
            }
            _ => return Err(bad("mode")),
        }
        if lisek_tv::TailMode::from_db(&s.tail_mode).is_none() {
            return Err(bad("tail_mode"));
        }
        if s.title_override.as_ref().is_some_and(|t| t.chars().count() > 200) {
            return Err(bad("title_override"));
        }
        out.push(s);
    }
    Ok(out)
}

/// A filler folder path as the libraries take them: absolute, normalized.
pub fn clean_folder(path: &str) -> Result<String, TvError> {
    lisek_db::normalize_root(path).ok_or_else(|| TvError::invalid("invalid_path", path.trim()))
}

pub fn clean_preset(mut preset: FillerPresetRow) -> Result<FillerPresetRow, TvError> {
    preset.name = preset.name.trim().to_string();
    if preset.name.is_empty() || preset.name.chars().count() > 100 {
        return Err(TvError::invalid("invalid_name", "1-100 characters"));
    }
    if !matches!(preset.kind.as_str(), "pre" | "mid" | "post" | "tail" | "fallback") {
        return Err(TvError::invalid("invalid_preset", "kind"));
    }
    match preset.mode.as_str() {
        "count" if preset.count.is_some_and(|c| (1..=50).contains(&c)) => {
            preset.duration_seconds = None;
            preset.pad_minutes = None;
        }
        "duration" if preset.duration_seconds.is_some_and(|d| (1..=86_400).contains(&d)) => {
            preset.count = None;
            preset.pad_minutes = None;
        }
        "pad" if preset.pad_minutes.is_some_and(|p| (1..=240).contains(&p)) => {
            preset.count = None;
            preset.duration_seconds = None;
        }
        _ => return Err(TvError::invalid("invalid_preset", "mode")),
    }
    preset.folder_ids.sort_unstable();
    preset.folder_ids.dedup();
    Ok(preset)
}

// ---- Channels ----------------------------------------------------------------

#[derive(Serialize, utoipa::ToSchema)]
pub struct AdminChannel {
    #[serde(flatten)]
    pub channel: db::TvChannel,
    pub built_until: Option<DateTime<Utc>>,
    pub last_build_at: Option<DateTime<Utc>>,
    /// Notes of the last build ("slot 2 has nothing to play").
    pub last_error: Option<String>,
    pub dirty: bool,
    pub slot_count: i64,
    /// The channel is being streamed now, and how many browser viewers it has.
    pub streaming: bool,
    pub viewers: i64,
    /// What plays right now.
    pub now_title: Option<String>,
    pub now_subtitle: Option<String>,
    pub now_ends_at: Option<DateTime<Utc>>,
}

#[utoipa::path(get,path="/api/admin/tv/channels",tag="admin",responses((status=200,body=Vec<AdminChannel>),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn list_channels(_admin: AdminUser, State(state): State<SharedState>) -> Result<Json<Vec<AdminChannel>>, TvError> {
    let pool = pool(&state)?;
    let channels = db::list_channels(&pool).await?;
    let statuses = db::statuses(&pool).await?;
    let counts: Vec<(Uuid, i64)> = sqlx_counts(&pool).await?;
    let running = state.tv.running();
    let now = Utc::now();
    let mut out = Vec::with_capacity(channels.len());
    for channel in channels {
        let status = statuses.iter().find(|s| s.channel_id == channel.id);
        let current = db::item_at(&pool, channel.id, now).await?;
        out.push(AdminChannel {
            slot_count: counts.iter().find(|c| c.0 == channel.id).map_or(0, |c| c.1),
            built_until: status.and_then(|s| s.built_until),
            last_build_at: status.and_then(|s| s.last_build_at),
            last_error: status.and_then(|s| s.last_error.clone()),
            dirty: status.is_none_or(|s| s.dirty),
            streaming: running.iter().any(|r| r.0 == channel.id),
            viewers: running.iter().find(|r| r.0 == channel.id).map_or(0, |r| r.1 as i64),
            now_title: current.as_ref().map(|c| c.title.clone()),
            now_subtitle: current.as_ref().and_then(|c| c.subtitle.clone()),
            now_ends_at: current.map(|c| c.ends_at),
            channel,
        });
    }
    Ok(Json(out))
}

async fn sqlx_counts(pool: &lisek_db::PgPool) -> Result<Vec<(Uuid, i64)>, TvError> {
    Ok(db::slot_counts(pool).await?)
}

#[utoipa::path(post,path="/api/admin/tv/channels",tag="admin",request_body=ChannelBody,responses((status=200,body=db::TvChannel),(status=400),(status=401),(status=403),(status=409,description="number_taken")),security(("lisek_auth"=[])))]
pub async fn create_channel(_admin: AdminUser, State(state): State<SharedState>, Json(body): Json<ChannelBody>) -> Result<Json<db::TvChannel>, TvError> {
    let pool = pool(&state)?;
    let mut input = clean_channel(body)?;
    if input.number.is_empty() {
        input.number = db::next_free_number(&pool).await?;
    }
    if db::number_taken(&pool, &input.number, None).await? {
        return Err(TvError::new(StatusCode::CONFLICT, "number_taken"));
    }
    check_filler(&pool, input.fallback_filler_id).await?;
    let channel = db::create_channel(&pool, &input).await?;
    tracing::info!(channel = %channel.name, number = %channel.number, "TV channel created");
    state.tv_wakeup.notify_one();
    Ok(Json(channel))
}

#[utoipa::path(put,path="/api/admin/tv/channels/{id}",tag="admin",params(("id"=Uuid,Path)),request_body=ChannelBody,responses((status=200,body=db::TvChannel),(status=400),(status=401),(status=403),(status=404),(status=409,description="number_taken")),security(("lisek_auth"=[])))]
pub async fn update_channel(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ChannelBody>,
) -> Result<Json<db::TvChannel>, TvError> {
    let pool = pool(&state)?;
    let mut input = clean_channel(body)?;
    let current = db::get_channel(&pool, id).await?.ok_or_else(not_found)?;
    if input.number.is_empty() {
        input.number = current.number.clone();
    }
    if db::number_taken(&pool, &input.number, Some(id)).await? {
        return Err(TvError::new(StatusCode::CONFLICT, "number_taken"));
    }
    check_filler(&pool, input.fallback_filler_id).await?;
    let channel = db::update_channel(&pool, id, &input).await?.ok_or_else(not_found)?;
    if channel.enabled {
        state.tv.restart_channel(id);
    } else {
        state.tv.stop_channel(id);
    }
    state.tv_wakeup.notify_one();
    Ok(Json(channel))
}

async fn check_filler(pool: &lisek_db::PgPool, preset: Option<Uuid>) -> Result<(), TvError> {
    if let Some(id) = preset
        && !db::list_presets(pool).await?.iter().any(|p| p.id == id)
    {
        return Err(TvError::invalid("filler_not_found", id.to_string()));
    }
    Ok(())
}

#[utoipa::path(delete,path="/api/admin/tv/channels/{id}",tag="admin",params(("id"=Uuid,Path)),responses((status=204),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn delete_channel(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<StatusCode, TvError> {
    let pool = pool(&state)?;
    let logo = db::delete_channel(&pool, id).await?.ok_or_else(not_found)?;
    state.tv.stop_channel(id);
    if let Some(path) = logo {
        let _ = tokio::fs::remove_file(path).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct OrderBody {
    pub ids: Vec<Uuid>,
}

#[utoipa::path(put,path="/api/admin/tv/channels/order",tag="admin",request_body=OrderBody,responses((status=204),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn reorder(_admin: AdminUser, State(state): State<SharedState>, Json(body): Json<OrderBody>) -> Result<StatusCode, TvError> {
    db::reorder_channels(&pool(&state)?, &body.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---- Logo ----------------------------------------------------------------------

/// Longest side of a stored logo, in pixels.
const LOGO_SIDE: u32 = 512;

/// A logo image decoded, fitted into `LOGO_SIDE` and encoded as PNG (so a
/// transparent background stays transparent).
pub fn process_logo(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    use image::{ImageFormat, ImageReader, Limits, imageops::FilterType};
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| "logo_invalid_image")?;
    if !matches!(reader.format(), Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP | ImageFormat::Gif)) {
        return Err("logo_invalid_image");
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(8000);
    limits.max_image_height = Some(8000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| "logo_invalid_image")?;
    let fitted = if decoded.width().max(decoded.height()) > LOGO_SIDE { decoded.resize(LOGO_SIDE, LOGO_SIDE, FilterType::Lanczos3) } else { decoded };
    let mut out = Vec::new();
    fitted.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|_| "logo_invalid_image")?;
    Ok(out)
}

fn logos_dir() -> PathBuf {
    crate::state::config_dir().join("tv").join("logos")
}

#[utoipa::path(post,path="/api/admin/tv/channels/{id}/logo",tag="admin",params(("id"=Uuid,Path)),request_body(content=Vec<u8>,content_type="image/*"),responses((status=204),(status=400,description="logo_invalid_image"),(status=401),(status=403),(status=404),(status=413)),security(("lisek_auth"=[])))]
pub async fn upload_logo(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>, body: Body) -> Result<StatusCode, TvError> {
    let pool = pool(&state)?;
    db::get_channel(&pool, id).await?.ok_or_else(not_found)?;
    let bytes = axum::body::to_bytes(body, MAX_LOGO_BYTES).await.map_err(|_| TvError::new(StatusCode::PAYLOAD_TOO_LARGE, "logo_too_large"))?;
    if bytes.is_empty() {
        return Err(TvError::new(StatusCode::BAD_REQUEST, "logo_empty"));
    }
    let png = tokio::task::spawn_blocking(move || process_logo(&bytes))
        .await
        .map_err(|_| TvError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error"))?
        .map_err(|code| TvError::new(StatusCode::BAD_REQUEST, code))?;
    let dir = logos_dir();
    let path = dir.join(format!("{id}.png"));
    let write = async {
        tokio::fs::create_dir_all(&dir).await?;
        let temporary = dir.join(format!("{id}.tmp"));
        tokio::fs::write(&temporary, &png).await?;
        tokio::fs::rename(&temporary, &path).await
    }
    .await;
    if let Err(e) = write {
        tracing::error!(error = %e, "saving a channel logo failed");
        return Err(TvError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error"));
    }
    db::set_logo_path(&pool, id, Some(&path.to_string_lossy())).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete,path="/api/admin/tv/channels/{id}/logo",tag="admin",params(("id"=Uuid,Path)),responses((status=204),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn delete_logo(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<StatusCode, TvError> {
    let pool = pool(&state)?;
    if let Some(path) = db::logo_path(&pool, id).await? {
        let _ = tokio::fs::remove_file(path).await;
    }
    if !db::set_logo_path(&pool, id, None).await? {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---- Schedule ------------------------------------------------------------------

#[utoipa::path(get,path="/api/admin/tv/channels/{id}/slots",tag="admin",params(("id"=Uuid,Path)),responses((status=200,body=Vec<db::Slot>),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn list_slots(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<Json<Vec<db::Slot>>, TvError> {
    let pool = pool(&state)?;
    db::get_channel(&pool, id).await?.ok_or_else(not_found)?;
    Ok(Json(db::list_slots(&pool, id).await?))
}

#[utoipa::path(put,path="/api/admin/tv/channels/{id}/slots",tag="admin",params(("id"=Uuid,Path)),request_body=Vec<SlotInput>,responses((status=200,body=Vec<db::Slot>),(status=400),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn replace_slots(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
    Json(slots): Json<Vec<SlotInput>>,
) -> Result<Json<Vec<db::Slot>>, TvError> {
    let pool = pool(&state)?;
    let slots = clean_slots(slots)?;
    let presets = db::list_presets(&pool).await?;
    for (i, s) in slots.iter().enumerate() {
        for filler in [s.pre_filler_id, s.mid_filler_id, s.post_filler_id, s.tail_filler_id].into_iter().flatten() {
            if !presets.iter().any(|p| p.id == filler) {
                return Err(TvError::invalid("invalid_slot", format!("{}: filler", i + 1)));
            }
        }
    }
    if !db::replace_slots(&pool, id, &slots).await? {
        return Err(not_found());
    }
    state.tv_wakeup.notify_one();
    Ok(Json(db::list_slots(&pool, id).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CopyBody {
    pub from_channel_id: Uuid,
}

#[utoipa::path(post,path="/api/admin/tv/channels/{id}/copy-schedule",tag="admin",params(("id"=Uuid,Path)),request_body=CopyBody,responses((status=200,body=Vec<db::Slot>),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn copy_schedule(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>, Json(body): Json<CopyBody>) -> Result<Json<Vec<db::Slot>>, TvError> {
    let pool = pool(&state)?;
    if body.from_channel_id == id || !db::copy_schedule(&pool, body.from_channel_id, id).await? {
        return Err(not_found());
    }
    state.tv_wakeup.notify_one();
    Ok(Json(db::list_slots(&pool, id).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SourceQuery {
    #[serde(default)]
    pub q: String,
}

#[utoipa::path(get,path="/api/admin/tv/sources",tag="admin",params(("q"=Option<String>,Query)),responses((status=200,body=Vec<db::SourceHit>),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn search_sources(_admin: AdminUser, State(state): State<SharedState>, Query(q): Query<SourceQuery>) -> Result<Json<Vec<db::SourceHit>>, TvError> {
    Ok(Json(db::search_sources(&pool(&state)?, &q.q, 30).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RebuildBody {
    pub mode: RebuildMode,
}

#[utoipa::path(post,path="/api/admin/tv/channels/{id}/rebuild",tag="admin",params(("id"=Uuid,Path)),request_body=RebuildBody,responses((status=200,body=db::RebuildResult),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn rebuild(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>, Json(body): Json<RebuildBody>) -> Result<Json<db::RebuildResult>, TvError> {
    let result = db::rebuild(&pool(&state)?, id, body.mode, Utc::now()).await?.ok_or_else(not_found)?;
    state.tv.restart_channel(id);
    Ok(Json(result))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PreviewQuery {
    #[serde(default = "default_hours")]
    pub hours: i64,
}

fn default_hours() -> i64 {
    12
}

#[utoipa::path(get,path="/api/admin/tv/channels/{id}/preview",tag="admin",params(("id"=Uuid,Path),("hours"=Option<i64>,Query)),responses((status=200,body=Vec<db::PlayoutEntry>),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn preview(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>, Query(q): Query<PreviewQuery>) -> Result<Json<Vec<db::PlayoutEntry>>, TvError> {
    let pool = pool(&state)?;
    db::get_channel(&pool, id).await?.ok_or_else(not_found)?;
    let now = Utc::now();
    Ok(Json(db::items_between(&pool, id, now, now + chrono::Duration::hours(q.hours.clamp(1, 72))).await?))
}

// ---- Fillers -------------------------------------------------------------------

#[utoipa::path(get,path="/api/admin/tv/fillers/folders",tag="admin",responses((status=200,body=Vec<db::FillerFolder>),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn list_folders(_admin: AdminUser, State(state): State<SharedState>) -> Result<Json<Vec<db::FillerFolder>>, TvError> {
    Ok(Json(db::list_folders(&pool(&state)?).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct FolderBody {
    pub path: String,
}

#[utoipa::path(post,path="/api/admin/tv/fillers/folders",tag="admin",request_body=FolderBody,responses((status=200,body=db::FillerFolder),(status=400),(status=401),(status=403),(status=409,description="folder_exists")),security(("lisek_auth"=[])))]
pub async fn add_folder(_admin: AdminUser, State(state): State<SharedState>, Json(body): Json<FolderBody>) -> Result<Json<db::FillerFolder>, TvError> {
    let pool = pool(&state)?;
    let path = clean_folder(&body.path)?;
    let folder = db::add_folder(&pool, &path).await?.ok_or(TvError::new(StatusCode::CONFLICT, "folder_exists"))?;
    spawn_scan(state.clone(), folder.id, path);
    Ok(Json(folder))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct FolderEnabled {
    pub enabled: bool,
}

#[utoipa::path(put,path="/api/admin/tv/fillers/folders/{id}",tag="admin",params(("id"=Uuid,Path)),request_body=FolderEnabled,responses((status=204),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn set_folder_enabled(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>, Json(body): Json<FolderEnabled>) -> Result<StatusCode, TvError> {
    if !db::set_folder_enabled(&pool(&state)?, id, body.enabled).await? {
        return Err(TvError::new(StatusCode::NOT_FOUND, "folder_not_found"));
    }
    state.tv_wakeup.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete,path="/api/admin/tv/fillers/folders/{id}",tag="admin",params(("id"=Uuid,Path)),responses((status=204),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn delete_folder(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<StatusCode, TvError> {
    if !db::delete_folder(&pool(&state)?, id).await? {
        return Err(TvError::new(StatusCode::NOT_FOUND, "folder_not_found"));
    }
    state.tv_wakeup.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post,path="/api/admin/tv/fillers/folders/{id}/scan",tag="admin",params(("id"=Uuid,Path)),responses((status=202),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn scan_folder(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<StatusCode, TvError> {
    let folder = db::get_folder(&pool(&state)?, id).await?.ok_or(TvError::new(StatusCode::NOT_FOUND, "folder_not_found"))?;
    spawn_scan(state, folder.id, folder.path);
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(get,path="/api/admin/tv/fillers/folders/{id}/files",tag="admin",params(("id"=Uuid,Path)),responses((status=200,body=Vec<db::FillerFileRow>),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn folder_files(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<Json<Vec<db::FillerFileRow>>, TvError> {
    Ok(Json(db::folder_files(&pool(&state)?, id).await?))
}

#[utoipa::path(get,path="/api/admin/tv/fillers/presets",tag="admin",responses((status=200,body=Vec<FillerPresetRow>),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn list_presets(_admin: AdminUser, State(state): State<SharedState>) -> Result<Json<Vec<FillerPresetRow>>, TvError> {
    Ok(Json(db::list_presets(&pool(&state)?).await?))
}

/// Creates a preset (no `id`) or changes one (`id`).
#[utoipa::path(post,path="/api/admin/tv/fillers/presets",tag="admin",request_body=FillerPresetRow,responses((status=200,body=FillerPresetRow),(status=400),(status=401),(status=403),(status=409,description="name_taken")),security(("lisek_auth"=[])))]
pub async fn save_preset(_admin: AdminUser, State(state): State<SharedState>, Json(preset): Json<FillerPresetRow>) -> Result<Json<FillerPresetRow>, TvError> {
    let pool = pool(&state)?;
    let preset = clean_preset(preset)?;
    let known: Vec<Uuid> = db::list_folders(&pool).await?.into_iter().map(|f| f.id).collect();
    if preset.folder_ids.iter().any(|f| !known.contains(f)) {
        return Err(TvError::invalid("folder_not_found", "folder_ids"));
    }
    let saved = db::save_preset(&pool, &preset).await?.ok_or(TvError::new(StatusCode::CONFLICT, "name_taken"))?;
    state.tv_wakeup.notify_one();
    Ok(Json(saved))
}

#[utoipa::path(delete,path="/api/admin/tv/fillers/presets/{id}",tag="admin",params(("id"=Uuid,Path)),responses((status=204),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn delete_preset(_admin: AdminUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<StatusCode, TvError> {
    if !db::delete_preset(&pool(&state)?, id).await? {
        return Err(TvError::new(StatusCode::NOT_FOUND, "preset_not_found"));
    }
    state.tv_wakeup.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

/// The video files under `root` (recursively), at most `MAX_FILLER_FILES`.
pub fn video_files(root: &FsPath) -> std::io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let Ok(entry) = entry else { continue };
            let Ok(kind) = entry.file_type() else { continue };
            let path = entry.path();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| FILLER_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
            {
                found.push(path);
                if found.len() >= MAX_FILLER_FILES {
                    return Ok(found);
                }
            }
        }
    }
    found.sort();
    Ok(found)
}

fn spawn_scan(state: SharedState, folder_id: Uuid, path: String) {
    tokio::spawn(async move {
        if let Err(e) = scan(&state, folder_id, &path).await {
            tracing::warn!(error = %e, folder = %path, "scanning a TV filler folder failed");
        }
    });
}

/// Lists a folder's videos, probes their duration, stores the result.
async fn scan(state: &SharedState, folder_id: Uuid, path: &str) -> Result<(), String> {
    let pool = state.pool().ok_or("server not ready")?;
    let root = PathBuf::from(path);
    let listed = tokio::task::spawn_blocking({
        let root = root.clone();
        move || video_files(&root)
    })
    .await
    .map_err(|e| e.to_string())?;
    let files = match listed {
        Ok(files) => files,
        Err(e) => {
            let message = if e.kind() == std::io::ErrorKind::NotFound { "folder_missing".to_string() } else { e.to_string() };
            db::replace_folder_files(&pool, folder_id, &[], Some(&message)).await.map_err(|e| e.to_string())?;
            return Ok(());
        }
    };
    let settings = lisek_db::playback::settings(&pool).await.map_err(|e| e.to_string())?;
    let mut probed: Vec<(String, i32)> = Vec::with_capacity(files.len());
    let mut failed = 0;
    for file in files {
        match lisek_playback::transcode::probe(&file, &settings).await {
            Ok(info) if info.duration >= 1.0 && info.duration < 6.0 * 3600.0 => {
                probed.push((file.to_string_lossy().into_owned(), (info.duration * 1000.0).round() as i32));
            }
            _ => failed += 1,
        }
    }
    let note = (failed > 0).then(|| format!("unreadable_files:{failed}"));
    tracing::info!(folder = %path, files = probed.len(), unreadable = failed, "TV filler folder scanned");
    db::replace_folder_files(&pool, folder_id, &probed, note.as_deref()).await.map_err(|e| e.to_string())?;
    state.tv_wakeup.notify_one();
    Ok(())
}

// ---- The builder ---------------------------------------------------------------

/// Keeps every enabled channel's timeline planned ahead: every ten minutes,
/// and straight away when an edit wakes it.
pub async fn run_builder(state: SharedState) {
    tokio::time::sleep(Duration::from_secs(5)).await;
    loop {
        if let Some(pool) = state.pool() {
            build_due(&pool, &state).await;
        }
        tokio::select! {
            () = tokio::time::sleep(BUILD_INTERVAL) => {}
            () = state.tv_wakeup.notified() => {
                // Several edits in a row become one pass.
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
    }
}

pub async fn build_due(pool: &lisek_db::PgPool, state: &SharedState) {
    let due = match db::builds_due(pool, Utc::now()).await {
        Ok(due) => due,
        Err(e) => {
            tracing::warn!(error = %e, "listing TV channels to plan failed");
            return;
        }
    };
    for (channel, mode) in due {
        match db::rebuild(pool, channel, mode, Utc::now()).await {
            Ok(Some(result)) => {
                tracing::info!(%channel, ?mode, planned = result.planned, "TV timeline planned");
                if mode != RebuildMode::Continue {
                    state.tv.restart_channel(channel);
                }
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(%channel, error = %e, "planning a TV timeline failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> ChannelBody {
        ChannelBody {
            number: "5.1".into(),
            name: "  Kino  ".into(),
            group_title: Some("  ".into()),
            enabled: true,
            timezone: "Europe/Warsaw".into(),
            quality: "720p".into(),
            fallback_filler_id: None,
            keep_running: false,
        }
    }

    #[test]
    fn channel_numbers_are_digits_with_up_to_two_decimals() {
        for ok in ["1", "5", "12345", "5.1", "5.12"] {
            assert!(valid_number(ok), "{ok}");
        }
        for bad in ["", "0.", ".5", "5.123", "123456", "a", "5,1", "-1", "5.1.2", " 5"] {
            assert!(!valid_number(bad), "{bad}");
        }
    }

    #[test]
    fn a_channel_is_trimmed_and_checked() {
        let clean = clean_channel(body()).unwrap();
        assert_eq!((clean.name.as_str(), clean.group_title, clean.number.as_str()), ("Kino", None, "5.1"));
        for (mutate, code) in [
            (Box::new(|b: &mut ChannelBody| b.name = " ".into()) as Box<dyn Fn(&mut ChannelBody)>, "invalid_name"),
            (Box::new(|b| b.number = "x".into()), "invalid_number"),
            (Box::new(|b| b.timezone = "Mars/Base".into()), "invalid_timezone"),
            (Box::new(|b| b.quality = "4k".into()), "invalid_quality"),
        ] {
            let mut b = body();
            mutate(&mut b);
            assert_eq!(clean_channel(b).err().map(|e| e.code), Some(code));
        }
        let mut empty_number = body();
        empty_number.number = String::new();
        assert!(clean_channel(empty_number).is_ok(), "an empty number means the next free one");
    }

    type Mutation = Box<dyn Fn(&mut SlotInput)>;

    fn slot() -> SlotInput {
        SlotInput {
            start_time: Some("20:30".into()),
            start_mode: "strict".into(),
            days_of_week: 127,
            source_kind: "library".into(),
            source_id: Uuid::new_v4(),
            play_order: "shuffle".into(),
            mode: "multiple".into(),
            count: Some(3),
            duration_minutes: Some(99),
            tail_mode: "offline".into(),
            pre_filler_id: None,
            mid_filler_id: None,
            post_filler_id: None,
            tail_filler_id: None,
            title_override: None,
            in_guide: true,
        }
    }

    #[test]
    fn slots_are_checked_and_unused_fields_cleared() {
        let cleaned = clean_slots(vec![slot()]).unwrap();
        assert_eq!(cleaned[0].duration_minutes, None, "a multiple slot has no duration");
        let mut flood = slot();
        flood.mode = "flood".into();
        assert_eq!(clean_slots(vec![flood]).unwrap()[0].count, None);
        let cases: Vec<Mutation> = vec![
            Box::new(|s| s.start_time = Some("25:00".into())),
            Box::new(|s| s.start_mode = "later".into()),
            Box::new(|s| s.days_of_week = 0),
            Box::new(|s| s.days_of_week = 128),
            Box::new(|s| s.source_kind = "folder".into()),
            Box::new(|s| s.play_order = "backwards".into()),
            Box::new(|s| s.count = None),
            Box::new(|s| s.count = Some(101)),
            Box::new(|s| {
                s.mode = "duration".into();
                s.duration_minutes = Some(0);
            }),
            Box::new(|s| s.mode = "loop".into()),
            Box::new(|s| s.tail_mode = "x".into()),
        ];
        for (i, mutate) in cases.iter().enumerate() {
            let mut s = slot();
            mutate(&mut s);
            let err = clean_slots(vec![slot(), s]).err().unwrap_or_else(|| panic!("case {i} should fail"));
            assert_eq!(err.code, "invalid_slot");
            assert!(err.detail.unwrap().starts_with("2:"), "the message names the slot");
        }
        assert!(clean_slots(vec![slot(); MAX_SLOTS + 1]).is_err());
    }

    #[test]
    fn filler_folders_are_absolute_paths_like_library_folders() {
        assert_eq!(clean_folder("/srv/ads/").unwrap(), "/srv/ads");
        assert!(clean_folder("ads/").is_err());
        assert!(clean_folder("   ").is_err());
        assert!(clean_folder("../etc").is_err());
    }

    #[test]
    fn presets_keep_only_the_fields_of_their_mode() {
        let preset = |mode: &str| FillerPresetRow {
            id: Uuid::nil(),
            name: " Ads ".into(),
            kind: "post".into(),
            mode: mode.into(),
            count: Some(2),
            duration_seconds: Some(60),
            pad_minutes: Some(30),
            folder_ids: vec![],
        };
        let pad = clean_preset(preset("pad")).unwrap();
        assert_eq!((pad.name.as_str(), pad.count, pad.duration_seconds, pad.pad_minutes), ("Ads", None, None, Some(30)));
        let mut bad = preset("pad");
        bad.pad_minutes = Some(0);
        assert!(clean_preset(bad).is_err());
        let mut kind = preset("count");
        kind.kind = "intro".into();
        assert!(clean_preset(kind).is_err());
        let mut dup = preset("count");
        let id = Uuid::new_v4();
        dup.folder_ids = vec![id, id];
        assert_eq!(clean_preset(dup).unwrap().folder_ids, vec![id]);
    }

    #[test]
    fn a_logo_is_fitted_and_other_files_are_refused() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(1200, 300, image::Rgba([200, 160, 77, 128])).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let out = process_logo(&png).unwrap();
        let decoded = image::load_from_memory(&out).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (512, 128), "fitted, aspect kept");
        assert!(image::load_from_memory(&out).unwrap().to_rgba8().pixels().all(|p| p[3] == 128), "transparency kept");
        assert_eq!(process_logo(b"not an image"), Err("logo_invalid_image"));
    }

    #[test]
    fn the_filler_scan_lists_videos_only() {
        let dir = std::env::temp_dir().join(format!("lisek-fillers-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in ["a.mp4", "b.MKV", "notes.txt", "sub/c.webm", "sub/d.jpg"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let found: Vec<String> = video_files(&dir).unwrap().iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(found, ["a.mp4", "b.MKV", "sub/c.webm"]);
        assert!(video_files(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
