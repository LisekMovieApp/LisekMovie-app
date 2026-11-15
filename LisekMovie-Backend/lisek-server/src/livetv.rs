//! Faza 20: Live TV (IPTV, ErsatzTV, Tvheadend …).
//!
//! A `live_tv` library is an M3U channel list plus an optional XMLTV guide
//! (http(s) links or files on the server). The server downloads both,
//! parses them (`livetv_parse`) and stores channels and programmes; it
//! refreshes them every `REFRESH_HOURS` and on demand.
//!
//! Playing a channel starts an FFmpeg process that reads the stream and
//! writes a live HLS playlist (a sliding window of short segments) into a
//! temporary folder; the browser plays it with hls.js like any other HLS
//! stream. The video is copied when the browser can decode it (H.264) and
//! transcoded to H.264 on request (`transcode: true`, which the player asks
//! for after a codec error). FFmpeg may only open network protocols, never
//! local files (`-protocol_whitelist`), whatever the playlist says. The
//! segments are served under an unguessable ticket, so `<video>` and hls.js
//! need no headers. A session nobody fetched for `IDLE_TIMEOUT` is stopped;
//! one account has one live stream at a time.

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lisek_db::livetv as db;

use crate::auth::{AdminUser, AuthUser};
use crate::livetv_parse;
use crate::state::SharedState;

const REFRESH_HOURS: i32 = 12;
const FETCH_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_PLAYLIST_BYTES: usize = 32 * 1024 * 1024;
const MAX_GUIDE_BYTES: usize = 256 * 1024 * 1024;
/// Guide window kept: from half a day ago to a week ahead.
const GUIDE_PAST_HOURS: i64 = 12;
const GUIDE_FUTURE_DAYS: i64 = 8;
const MAX_GUIDE_HOURS: i64 = 48;
const PROTOCOLS: &str = "http,https,tcp,tls,crypto,udp,rtp,rtsp,rtmp,rtmps,rtmpt,httpproxy";

pub struct LiveError(StatusCode, &'static str);
impl IntoResponse for LiveError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error_code": self.1 }))).into_response()
    }
}
impl From<lisek_db::DbError> for LiveError {
    fn from(e: lisek_db::DbError) -> Self {
        tracing::error!(error = %e, "live TV database error");
        LiveError(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
    }
}
fn pool(state: &SharedState) -> Result<lisek_db::PgPool, LiveError> {
    state.pool().ok_or(LiveError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))
}
fn not_found() -> LiveError {
    LiveError(StatusCode::NOT_FOUND, "channel_not_found")
}

// ---- Sources and refresh -----------------------------------------------------

/// An http(s) link or an absolute path on the server.
pub fn valid_source(value: &str) -> bool {
    let value = value.trim();
    if let Ok(url) = reqwest::Url::parse(value)
        && matches!(url.scheme(), "http" | "https")
    {
        return url.host_str().is_some();
    }
    std::path::Path::new(value).is_absolute()
}

async fn fetch(source: &str, limit: usize) -> Result<Vec<u8>, String> {
    let source = source.trim();
    if source.starts_with("http://") || source.starts_with("https://") {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        let mut response = client.get(source).send().await.map_err(|e| format!("download failed: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("the server answered {}", response.status()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| format!("download failed: {e}"))? {
            if bytes.len() + chunk.len() > limit {
                return Err("the file is too large".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    } else {
        let metadata = tokio::fs::metadata(source).await.map_err(|e| format!("cannot read {source}: {e}"))?;
        if metadata.len() > limit as u64 {
            return Err("the file is too large".to_string());
        }
        tokio::fs::read(source).await.map_err(|e| format!("cannot read {source}: {e}"))
    }
}

/// Downloads and stores one library's channels and guide. A failure keeps
/// the previous ones and is recorded as `last_error`.
pub async fn refresh(pool: &lisek_db::PgPool, library_id: Uuid) -> Result<(usize, usize), String> {
    let result = refresh_inner(pool, library_id).await;
    match &result {
        Ok((channels, programs)) => tracing::info!(%library_id, channels, programs, "live TV refreshed"),
        Err(error) => {
            tracing::warn!(%library_id, %error, "live TV refresh failed");
            let _ = db::record_error(pool, library_id, error).await;
        }
    }
    result
}

async fn refresh_inner(pool: &lisek_db::PgPool, library_id: Uuid) -> Result<(usize, usize), String> {
    let source = db::get_source(pool, library_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("this library has no channel list")?;
    let playlist = fetch(&source.m3u_url, MAX_PLAYLIST_BYTES).await.map_err(|e| format!("channel list: {e}"))?;
    let playlist = String::from_utf8_lossy(&playlist);
    let channels = livetv_parse::parse_m3u(&playlist);
    if channels.is_empty() {
        return Err("the channel list has no playable channels".to_string());
    }
    let mut programs = Vec::new();
    if let Some(guide_url) = source.xmltv_url.as_deref().filter(|u| !u.trim().is_empty()) {
        let guide = fetch(guide_url, MAX_GUIDE_BYTES).await.map_err(|e| format!("guide: {e}"))?;
        let guide = livetv_parse::maybe_gunzip(guide, MAX_GUIDE_BYTES).map_err(|e| format!("guide: {e}"))?;
        let now = Utc::now();
        let from = now - chrono::Duration::hours(GUIDE_PAST_HOURS);
        let to = now + chrono::Duration::days(GUIDE_FUTURE_DAYS);
        let parse_channels = channels.clone();
        programs = tokio::task::spawn_blocking(move || livetv_parse::parse_xmltv(&guide, &parse_channels, from, to))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("guide: {e}"))?;
    }
    db::replace_catalog(pool, library_id, &channels, &programs).await.map_err(|e| e.to_string())
}

/// Refreshes sources older than `REFRESH_HOURS`, checking every 30 minutes.
pub async fn run(state: SharedState) {
    loop {
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        let Some(pool) = state.pool() else { continue };
        let Ok(due) = db::sources_due(&pool, REFRESH_HOURS).await else { continue };
        for library_id in due {
            let _ = refresh(&pool, library_id).await;
        }
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetSource {
    /// http(s) link to the M3U playlist, or an absolute path on the server.
    pub m3u_url: String,
    /// XMLTV guide (optional; `.xml` or `.xml.gz`).
    pub xmltv_url: Option<String>,
}

/// Every Live TV library with its source and the last refresh.
#[utoipa::path(get,path="/api/admin/livetv",tag="admin",responses((status=200,body=Vec<db::LiveTvSource>),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn admin_sources(_admin: AdminUser, State(state): State<SharedState>) -> Result<Json<Vec<db::LiveTvSource>>, LiveError> {
    Ok(Json(db::list_sources(&pool(&state)?).await?))
}

/// The channels of one Live TV library as the administrator edits them.
#[utoipa::path(get,path="/api/admin/livetv/{library_id}/channels",tag="admin",params(("library_id"=Uuid,Path)),responses((status=200,body=Vec<db::AdminLiveChannel>),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn admin_channels(_admin: AdminUser, State(state): State<SharedState>, Path(library_id): Path<Uuid>) -> Result<Json<Vec<db::AdminLiveChannel>>, LiveError> {
    Ok(Json(db::admin_channels(&pool(&state)?, library_id).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ChannelSettings {
    pub enabled: bool,
    /// Replaces the provider's number in the lists and the IPTV export; empty clears it.
    #[serde(default)]
    pub number_override: Option<String>,
    #[serde(default)]
    pub group_override: Option<String>,
    /// A logo address (http/https) replacing the provider's; empty clears it.
    #[serde(default)]
    pub logo_override: Option<String>,
}

fn clean_override(value: Option<String>, max: usize) -> Result<Option<String>, LiveError> {
    let value = value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    if value.as_ref().is_some_and(|v| v.chars().count() > max) {
        return Err(LiveError(StatusCode::BAD_REQUEST, "invalid_channel_settings"));
    }
    Ok(value)
}

/// Checked settings: enabled, number, group, logo.
type Cleaned = (bool, Option<String>, Option<String>, Option<String>);

pub fn clean_settings(s: ChannelSettings) -> Result<Cleaned, LiveError> {
    let number = clean_override(s.number_override, 20)?;
    let group = clean_override(s.group_override, 100)?;
    let logo = clean_override(s.logo_override, 500)?;
    if let Some(logo) = &logo
        && !reqwest::Url::parse(logo).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
    {
        return Err(LiveError(StatusCode::BAD_REQUEST, "invalid_channel_settings"));
    }
    Ok((s.enabled, number, group, logo))
}

/// Turns a channel on or off and overrides its number, group and logo.
#[utoipa::path(put,path="/api/admin/livetv/channels/{channel_id}",tag="admin",params(("channel_id"=Uuid,Path)),request_body=ChannelSettings,responses((status=204),(status=400,description="invalid_channel_settings"),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn admin_set_channel(_admin: AdminUser, State(state): State<SharedState>, Path(channel_id): Path<Uuid>, Json(settings): Json<ChannelSettings>) -> Result<StatusCode, LiveError> {
    let (enabled, number, group, logo) = clean_settings(settings)?;
    if !db::update_channel_settings(&pool(&state)?, channel_id, enabled, number.as_deref(), group.as_deref(), logo.as_deref()).await? {
        return Err(not_found());
    }
    if !enabled {
        state.tv.stop_channel(channel_id);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub fn check_source(req: &SetSource) -> Result<(), LiveError> {
    let invalid = || LiveError(StatusCode::BAD_REQUEST, "invalid_live_tv_source");
    if !valid_source(&req.m3u_url) {
        return Err(invalid());
    }
    if let Some(xmltv) = req.xmltv_url.as_deref().filter(|u| !u.trim().is_empty())
        && !valid_source(xmltv)
    {
        return Err(invalid());
    }
    Ok(())
}

/// Changes a Live TV library's links and refreshes it.
#[utoipa::path(put,path="/api/admin/livetv/{library_id}",tag="admin",params(("library_id"=Uuid,Path)),request_body=SetSource,responses((status=200,body=db::LiveTvSource),(status=400),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn admin_set_source(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
    Json(req): Json<SetSource>,
) -> Result<Json<db::LiveTvSource>, LiveError> {
    check_source(&req)?;
    let pool = pool(&state)?;
    let library = lisek_db::get_library(&pool, library_id).await?;
    if library.is_none_or(|l| l.library_type != db::LIVE_TV) {
        return Err(LiveError(StatusCode::NOT_FOUND, "library_not_found"));
    }
    let xmltv = req.xmltv_url.as_deref().map(str::trim).filter(|u| !u.is_empty());
    db::set_source(&pool, library_id, req.m3u_url.trim(), xmltv).await?;
    tracing::info!(%library_id, admin = %auth.user.username, "live TV source changed");
    let _ = refresh(&pool, library_id).await;
    db::get_source(&pool, library_id).await?.map(Json).ok_or(LiveError(StatusCode::NOT_FOUND, "library_not_found"))
}

/// Downloads the channel list and guide again now.
#[utoipa::path(post,path="/api/admin/livetv/{library_id}/refresh",tag="admin",params(("library_id"=Uuid,Path)),responses((status=200,body=db::LiveTvSource),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn admin_refresh(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<db::LiveTvSource>, LiveError> {
    let pool = pool(&state)?;
    if db::get_source(&pool, library_id).await?.is_none() {
        return Err(LiveError(StatusCode::NOT_FOUND, "library_not_found"));
    }
    let _ = refresh(&pool, library_id).await;
    db::get_source(&pool, library_id).await?.map(Json).ok_or(LiveError(StatusCode::NOT_FOUND, "library_not_found"))
}

// ---- Channels and guide --------------------------------------------------------

/// The Live TV libraries `auth` may open: an age limit hides Live TV (it
/// carries no ratings), a library restriction is honoured. `None`: all.
async fn visible_libraries(pool: &lisek_db::PgPool, auth: &AuthUser) -> Result<Option<Vec<Uuid>>, LiveError> {
    if auth.access.max_age_rating.is_some() {
        return Ok(Some(Vec::new()));
    }
    Ok(match &auth.access.libraries {
        None => None,
        Some(allowed) => {
            let live: Vec<Uuid> = db::list_sources(pool).await?.into_iter().map(|s| s.library_id).collect();
            Some(allowed.iter().copied().filter(|id| live.contains(id)).collect())
        }
    })
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct ChannelResponse {
    #[serde(flatten)]
    pub channel: db::LiveChannel,
    /// `/api/livetv/channels/{id}/logo`, or null.
    pub logo_url: Option<String>,
    pub now: Option<db::LiveProgram>,
    pub next: Option<db::LiveProgram>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LiveTvStatus {
    /// Whether this account has any channel to watch.
    pub available: bool,
    pub channel_count: usize,
}

/// Whether Live TV is there for this account (the main menu's tab).
#[utoipa::path(get,path="/api/livetv/status",tag="media",responses((status=200,body=LiveTvStatus),(status=401)),security(("lisek_auth"=[])))]
pub async fn status(auth: AuthUser, State(state): State<SharedState>) -> Result<Json<LiveTvStatus>, LiveError> {
    let pool = pool(&state)?;
    let libraries = visible_libraries(&pool, &auth).await?;
    let count = db::channels(&pool, libraries.as_deref()).await?.len();
    Ok(Json(LiveTvStatus { available: count > 0, channel_count: count }))
}

fn logo_url(channel: &db::LiveChannel) -> Option<String> {
    channel.has_logo.then(|| format!("/api/livetv/channels/{}/logo", channel.id))
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct GuideQuery {
    /// Window start (RFC 3339); default: half an hour ago.
    pub start: Option<DateTime<Utc>>,
    /// Window length in hours (1–48); default 6.
    pub hours: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct GuideChannel {
    #[serde(flatten)]
    pub channel: db::LiveChannel,
    pub logo_url: Option<String>,
    pub programs: Vec<db::LiveProgram>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct GuideResponse {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub now: DateTime<Utc>,
    pub channels: Vec<GuideChannel>,
}

/// The programme guide: every channel with its programmes in a window.
#[utoipa::path(get,path="/api/livetv/guide",tag="media",params(GuideQuery),responses((status=200,body=GuideResponse),(status=401)),security(("lisek_auth"=[])))]
pub async fn guide(
    auth: AuthUser,
    State(state): State<SharedState>,
    Query(query): Query<GuideQuery>,
) -> Result<Json<GuideResponse>, LiveError> {
    let pool = pool(&state)?;
    let libraries = visible_libraries(&pool, &auth).await?;
    let now = Utc::now();
    let start = query.start.unwrap_or(now - chrono::Duration::minutes(30));
    let hours = query.hours.unwrap_or(6).clamp(1, MAX_GUIDE_HOURS);
    let end = start + chrono::Duration::hours(hours);
    let channels = db::channels(&pool, libraries.as_deref()).await?;
    let mut by_channel: HashMap<Uuid, Vec<db::LiveProgram>> = HashMap::new();
    for program in db::programs(&pool, libraries.as_deref(), start, end).await? {
        by_channel.entry(program.channel_id).or_default().push(program);
    }
    let channels = channels
        .into_iter()
        .map(|channel| GuideChannel {
            logo_url: logo_url(&channel),
            programs: by_channel.remove(&channel.id).unwrap_or_default(),
            channel,
        })
        .collect();
    Ok(Json(GuideResponse { start, end, now, channels }))
}

/// Every channel with what is on now and next.
#[utoipa::path(get,path="/api/livetv/channels",tag="media",responses((status=200,body=Vec<ChannelResponse>),(status=401)),security(("lisek_auth"=[])))]
pub async fn channels(auth: AuthUser, State(state): State<SharedState>) -> Result<Json<Vec<ChannelResponse>>, LiveError> {
    let pool = pool(&state)?;
    let libraries = visible_libraries(&pool, &auth).await?;
    let now = Utc::now();
    let channels = db::channels(&pool, libraries.as_deref()).await?;
    let mut by_channel: HashMap<Uuid, Vec<db::LiveProgram>> = HashMap::new();
    for program in db::programs(&pool, libraries.as_deref(), now, now + chrono::Duration::hours(12)).await? {
        by_channel.entry(program.channel_id).or_default().push(program);
    }
    Ok(Json(
        channels
            .into_iter()
            .map(|channel| {
                let mut upcoming = by_channel.remove(&channel.id).unwrap_or_default().into_iter();
                let first = upcoming.next();
                let (now_on, next) = match first {
                    Some(p) if p.starts_at <= now => (Some(p), upcoming.next()),
                    other => (None, other),
                };
                ChannelResponse { logo_url: logo_url(&channel), now: now_on, next, channel }
            })
            .collect(),
    ))
}

async fn visible_channel(pool: &lisek_db::PgPool, auth: &AuthUser, channel_id: Uuid) -> Result<db::ChannelSource, LiveError> {
    let channel = db::channel_source(pool, channel_id).await?.ok_or_else(not_found)?;
    let allowed = match visible_libraries(pool, auth).await? {
        None => true,
        Some(libraries) => libraries.contains(&channel.library_id),
    };
    if allowed { Ok(channel) } else { Err(not_found()) }
}

/// A channel's logo, fetched once and kept in `<config>/livetv/logos`.
#[utoipa::path(get,path="/api/livetv/channels/{channel_id}/logo",tag="media",params(("channel_id"=Uuid,Path)),responses((status=200,description="Image"),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn logo(auth: AuthUser, State(state): State<SharedState>, Path(channel_id): Path<Uuid>) -> Result<Response, LiveError> {
    let pool = pool(&state)?;
    let channel = visible_channel(&pool, &auth, channel_id).await?;
    let Some(url) = channel.logo_url else { return Err(not_found()) };
    let key = crate::http_cache::etag(url.as_bytes()).replace('"', "");
    let path = crate::state::config_dir().join("livetv").join("logos").join(format!("{key}.png"));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let raw = fetch(&url, 5 * 1024 * 1024).await.map_err(|_| not_found())?;
            let png = tokio::task::spawn_blocking(move || -> Option<Vec<u8>> {
                let picture = image::load_from_memory(&raw).ok()?;
                let picture = if picture.width() > 512 || picture.height() > 512 {
                    picture.resize(512, 512, image::imageops::FilterType::Lanczos3)
                } else {
                    picture
                };
                let mut out = std::io::Cursor::new(Vec::new());
                picture.to_rgba8().write_to(&mut out, image::ImageFormat::Png).ok()?;
                Some(out.into_inner())
            })
            .await
            .ok()
            .flatten()
            .ok_or_else(not_found)?;
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = tokio::fs::write(&path, &png).await;
            png
        }
    };
    Ok(crate::media::image_response(bytes, "image/png"))
}

// ---- Live streams ----------------------------------------------------------------

/// FFmpeg arguments for one live stream into `dir`.
pub fn ffmpeg_args(stream_url: &str, dir: &std::path::Path, transcode: bool) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "warning"].map(String::from).to_vec();
    args.extend(["-protocol_whitelist".into(), PROTOCOLS.into()]);
    if stream_url.starts_with("http://") || stream_url.starts_with("https://") {
        args.extend(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "5"].map(String::from));
    }
    args.extend(["-rw_timeout".into(), "15000000".into()]);
    args.extend(["-fflags".into(), "+genpts+discardcorrupt".into(), "-i".into(), stream_url.into()]);
    args.extend(["-map", "0:v:0?", "-map", "0:a:0?", "-sn", "-dn"].map(String::from));
    if transcode {
        args.extend(
            [
                "-c:v", "libx264", "-preset", "veryfast", "-tune", "zerolatency", "-profile:v", "high",
                "-pix_fmt", "yuv420p", "-vf", "scale=-2:'min(720,ih)'", "-b:v", "3500k", "-maxrate", "4000k",
                "-bufsize", "8000k", "-g", "50", "-keyint_min", "50", "-sc_threshold", "0",
            ]
            .map(String::from),
        );
    } else {
        args.extend(["-c:v", "copy"].map(String::from));
    }
    args.extend(["-c:a", "aac", "-b:a", "160k", "-ac", "2"].map(String::from));
    args.extend(
        [
            "-f", "hls", "-hls_time", "4", "-hls_list_size", "8",
            "-hls_flags", "delete_segments+independent_segments+omit_endlist",
        ]
        .map(String::from),
    );
    args.push("-hls_segment_filename".into());
    args.push(dir.join("seg%06d.ts").to_string_lossy().into_owned());
    args.push(dir.join("index.m3u8").to_string_lossy().into_owned());
    args
}

#[derive(Deserialize, Default, utoipa::ToSchema)]
pub struct PlayRequest {
    /// Re-encode the video to H.264 (for streams the browser cannot decode).
    #[serde(default)]
    pub transcode: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PlayResponse {
    pub session_id: Uuid,
    /// Live HLS playlist (no headers needed).
    pub stream_url: String,
    pub channel_id: Uuid,
    pub name: String,
    pub transcoding: bool,
}

/// Starts watching a channel. Answers once the first segment is ready. The
/// stream is shared: every viewer of a channel gets the same FFmpeg.
#[utoipa::path(post,path="/api/livetv/channels/{channel_id}/play",tag="media",params(("channel_id"=Uuid,Path)),request_body=PlayRequest,responses((status=200,body=PlayResponse),(status=401),(status=404),(status=429,description="live_capacity_reached"),(status=502,description="live_stream_unavailable"),(status=503)),security(("lisek_auth"=[])))]
pub async fn play(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(channel_id): Path<Uuid>,
    body: Option<Json<PlayRequest>>,
) -> Result<Json<PlayResponse>, LiveError> {
    let request = body.map(|Json(b)| b).unwrap_or_default();
    let pool = pool(&state)?;
    let channel = visible_channel(&pool, &auth, channel_id).await?;
    if !livetv_parse::allowed_stream_url(&channel.stream_url) {
        return Err(LiveError(StatusCode::BAD_GATEWAY, "live_stream_unavailable"));
    }
    let started = crate::tvlive::ensure_iptv(&state, channel.id, &channel.name, &channel.stream_url, request.transcode).await;
    let _ = db::record_outcome(&pool, channel.id, started.is_ok()).await;
    started.map_err(|e| match e {
        crate::tvlive::StreamError::Capacity => LiveError(StatusCode::TOO_MANY_REQUESTS, "live_capacity_reached"),
        crate::tvlive::StreamError::Unavailable => LiveError(StatusCode::BAD_GATEWAY, "live_stream_unavailable"),
    })?;
    // One channel per account: switching replaces the earlier one.
    let (session_id, ticket) = state.tv.add_viewer(auth.user.id, channel.id);
    tracing::info!(channel = %channel.name, user = %auth.user.username, transcode = request.transcode, "live stream opened");
    Ok(Json(PlayResponse {
        stream_url: format!("/api/tv/stream/{session_id}/{ticket}/index.m3u8"),
        session_id,
        channel_id,
        name: channel.name,
        transcoding: request.transcode,
    }))
}

/// Stops watching (leaving the player). The stream itself stops when nobody
/// is left.
#[utoipa::path(delete,path="/api/livetv/sessions/{session_id}",tag="media",params(("session_id"=Uuid,Path)),responses((status=204),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn stop(auth: AuthUser, State(state): State<SharedState>, Path(session_id): Path<Uuid>) -> StatusCode {
    if state.tv.stop_viewer(session_id, auth.user.id) { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_links_or_server_paths() {
        assert!(valid_source("https://iptv.example/list.m3u"));
        assert!(valid_source("http://192.168.1.10:8409/iptv/channels.m3u"));
        assert!(valid_source(if cfg!(windows) { "C:\\tv\\list.m3u" } else { "/srv/tv/list.m3u" }));
        assert!(!valid_source("ftp://x/list.m3u"));
        assert!(!valid_source("list.m3u"));
    }

    #[test]
    fn ffmpeg_reads_only_network_protocols_and_writes_live_hls() {
        let dir = std::path::Path::new("/tmp/live");
        let args = ffmpeg_args("http://iptv/1.ts", dir, false);
        let joined = args.join(" ");
        let whitelist = args.iter().position(|a| a == "-protocol_whitelist").unwrap();
        assert!(!args[whitelist + 1].split(',').any(|p| p == "file" || p == "concat" || p == "pipe"));
        assert!(whitelist < args.iter().position(|a| a == "-i").unwrap(), "applies to the input");
        assert!(joined.contains("-c:v copy"));
        assert!(joined.contains("-reconnect 1"));
        assert!(joined.contains("delete_segments"));
        assert!(args.last().unwrap().ends_with("index.m3u8"));
        let transcode = ffmpeg_args("udp://239.0.0.1:1234", dir, true).join(" ");
        assert!(transcode.contains("libx264") && !transcode.contains("-reconnect"));
    }
}
