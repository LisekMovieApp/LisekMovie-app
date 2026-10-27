//! Playback endpoints. Starting playback decides per stream how the file
//! reaches the client (direct play, remux, transcode; see
//! `lisek_playback::decision`); remux and transcode run as HLS sessions
//! (`lisek_playback::session`). Media URLs carry a revocable, item-scoped
//! ticket instead of the session token, because `<video>`, hls.js and
//! `<track>` cannot send headers.

use std::collections::HashSet;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{LazyLock, Mutex};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use lisek_core::media_probe::{MediaProbe, StreamKind};
use lisek_core::playback_settings::PlaybackSettings;
use lisek_db::playback as db;
use lisek_db::streams::{self, FileStamp};
use lisek_playback::decision::{
    self, AudioAction, DecisionInput, PlayMethod, PlaybackPlan, PlaybackRequest, SubtitleDelivery,
    TranscodeReason, VideoAction,
};
use lisek_playback::downmix::{DownmixAlgorithm, DownmixPreference};
use lisek_playback::hls::{HlsFile, Rendition};
use lisek_playback::keyframes::{self, SegmentMap};
use lisek_playback::profile::ClientCapabilities;
use lisek_playback::session::{self, SessionError, SessionSpec};
use lisek_playback::subtitles::{self, ExternalSubtitle, SubtitleKind, TrackRef};
use lisek_playback::transcode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{
    auth::{AdminUser, AuthUser},
    state::SharedState,
};

pub struct PlaybackError(pub(crate) StatusCode, pub(crate) &'static str);
impl IntoResponse for PlaybackError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error_code": self.1, "message": "The requested playback action could not be completed."}))).into_response()
    }
}
impl From<lisek_db::DbError> for PlaybackError {
    fn from(err: lisek_db::DbError) -> Self {
        tracing::error!(error = %err, "playback database operation failed");
        Self(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
    }
}
impl From<SessionError> for PlaybackError {
    fn from(err: SessionError) -> Self {
        let status = match err {
            SessionError::NotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        };
        Self(status, err.code())
    }
}
pub(crate) async fn pool(state: &SharedState) -> Result<lisek_db::PgPool, PlaybackError> {
    state.pool().ok_or(PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))
}
pub(crate) async fn checked_file(source: &db::Source) -> Result<PathBuf, PlaybackError> {
    let missing = || PlaybackError(StatusCode::NOT_FOUND, "media_file_unavailable");
    let path = tokio::fs::canonicalize(&source.file_path).await.map_err(|_| missing())?;
    for root in &source.root_paths {
        if let Ok(root) = tokio::fs::canonicalize(root).await
            && path.starts_with(root)
            && tokio::fs::metadata(&path).await.map(|m| m.is_file()).unwrap_or(false)
        {
            return Ok(path);
        }
    }
    Err(missing())
}

async fn enforce_safety(pool: &lisek_db::PgPool, id: Uuid, info: &transcode::SourceInfo) -> Result<(), PlaybackError> {
    let config = db::safety(pool).await?;
    let policy = lisek_playback::TranscodeSafety {
        enabled: config.enabled,
        max_width: config.max_width as u32,
        max_height: config.max_height as u32,
        max_bitrate_kbps: config.max_bitrate_kbps.map(|v| v as u32),
    };
    if let Some(reason) = policy.rejection(info.width, info.height, info.bitrate_kbps) {
        let clamp = |v: u32| v.min(i32::MAX as u32) as i32;
        db::record_kill(pool, id, reason, clamp(info.width), clamp(info.height), clamp(info.bitrate_kbps)).await?;
        return Err(PlaybackError(StatusCode::FORBIDDEN, "transcode_limit_exceeded"));
    }
    Ok(())
}

/// The file's probe: stored by the scanner (or an earlier playback) while
/// the file's size and mtime are unchanged and the probe has the fields
/// playback needs; otherwise ffprobe runs once and the result is stored.
/// Direct play with a remote bitrate limit calls this on every range
/// request, so before this cache each chunk of a movie spawned ffprobe.
async fn media_probe(pool: &lisek_db::PgPool, item_id: Uuid, path: &FsPath, settings: &PlaybackSettings) -> Result<MediaProbe, String> {
    let stamp = tokio::fs::metadata(path).await.ok().map(|m| FileStamp::from_metadata(&m));
    match streams::stored_probe(pool, item_id).await {
        Ok(Some((probe, stored))) if !stamp.is_some_and(|s| s.differs_from(&stored)) && probe.start_time_seconds.is_some() => {
            return Ok(probe);
        }
        Ok(_) => {}
        Err(err) => tracing::warn!(%item_id, %err, "cannot read stored probe; probing the file"),
    }
    let probe = transcode::probe_media(path, settings).await?;
    if let Err(err) = streams::save_probe(pool, item_id, &probe, stamp).await {
        tracing::warn!(%item_id, %err, "cannot store probe result");
    }
    Ok(probe)
}

/// Keyframes of the primary video for cutting a copied stream: Matroska
/// Cues (read on the spot), or a stored ffprobe scan.
async fn keyframes_for(pool: &lisek_db::PgPool, item_id: Uuid, path: &FsPath, probe: &MediaProbe) -> Option<Vec<f64>> {
    if probe.container.as_deref().is_some_and(|c| c.contains("matroska")) {
        let file = path.to_owned();
        match tokio::task::spawn_blocking(move || keyframes::matroska_keyframes(&file)).await {
            Ok(Ok(Some(found))) => return Some(found),
            Ok(Ok(None)) => tracing::info!(%item_id, "Matroska file without usable Cues"),
            Ok(Err(err)) => tracing::warn!(%item_id, %err, "cannot read Matroska Cues"),
            Err(_) => {}
        }
    }
    let stamp = tokio::fs::metadata(path).await.ok().map(|m| FileStamp::from_metadata(&m));
    streams::stored_keyframes(pool, item_id, stamp).await.ok().flatten()
}

/// Scans a file's keyframes in the background (one file at a time: the
/// scan reads the whole file) so its next playback can be a remux.
fn schedule_keyframe_scan(state: SharedState, item_id: Uuid, path: PathBuf, video_index: i32, settings: PlaybackSettings) {
    static SCANNING: LazyLock<Mutex<HashSet<Uuid>>> = LazyLock::new(Default::default);
    static ONE_AT_A_TIME: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
    if !SCANNING.lock().unwrap_or_else(|e| e.into_inner()).insert(item_id) {
        return;
    }
    tokio::spawn(async move {
        if let Ok(_permit) = ONE_AT_A_TIME.acquire().await {
            let stamp = tokio::fs::metadata(&path).await.ok().map(|m| FileStamp::from_metadata(&m));
            let started = std::time::Instant::now();
            match keyframes::scan_with_ffprobe(&path, video_index, &settings).await {
                Ok(found) => {
                    tracing::info!(%item_id, keyframes = found.len(), seconds = started.elapsed().as_secs(), "keyframe index built");
                    if let Some(pool) = state.pool()
                        && let Err(err) = streams::save_keyframes(&pool, item_id, stamp, &found).await
                    {
                        tracing::warn!(%item_id, %err, "cannot store keyframes");
                    }
                }
                Err(reason) => tracing::warn!(%item_id, %reason, "keyframe scan failed"),
            }
        }
        SCANNING.lock().unwrap_or_else(|e| e.into_inner()).remove(&item_id);
    });
}

/// The client's choices when starting playback. Every field is optional;
/// an empty body plays the default tracks the way the client's family
/// handles best.
#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct StartRequest {
    /// What the client decodes. Omitted: the preset of its browser/TV
    /// family, recognised from the User-Agent.
    pub client: Option<ClientCapabilities>,
    /// ffprobe index of the audio stream (see `audio_tracks`).
    pub audio_stream_index: Option<i32>,
    /// Subtitle track id from `subtitle_tracks`: `s<index>` embedded,
    /// `x<n>` a file next to the video.
    pub subtitle: Option<String>,
    /// Burn the subtitle into the picture even when it is text (keeps ASS
    /// styling; costs a transcode). Picture subtitles are always burnt.
    pub burn_subtitle: bool,
    pub force_transcode: bool,
    /// Start here instead of at the saved resume position.
    pub start_position_seconds: Option<f64>,
    pub max_height: Option<u32>,
    pub max_bitrate_kbps: Option<u32>,
    pub downmix: DownmixPreference,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct AudioTrack {
    pub index: i32,
    pub codec: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    pub channels: Option<i32>,
    pub channel_layout: Option<String>,
    pub is_default: bool,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SubtitleTrack {
    pub id: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    #[schema(value_type = String)]
    pub kind: SubtitleKind,
    pub forced: bool,
    pub is_default: bool,
    /// A file next to the video rather than a stream inside it.
    pub external: bool,
    /// WebVTT for a `<track>` element (direct play only).
    pub url: Option<String>,
    /// Position among the HLS subtitle renditions (hls.js
    /// `subtitleTracks`), for switching without restarting playback.
    pub hls_rendition: Option<u32>,
}

/// What actually reaches the client.
#[derive(Serialize, utoipa::ToSchema)]
pub struct DeliveredStreams {
    pub video_codec: String,
    pub width: u32,
    pub height: u32,
    /// The video stream is passed through untouched.
    pub video_copied: bool,
    pub tone_mapped: bool,
    pub audio_codec: Option<String>,
    pub audio_channels: Option<u32>,
    pub audio_copied: bool,
    #[schema(value_type = Option<String>)]
    pub downmix: Option<DownmixAlgorithm>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct StartResponse {
    pub session_id: Uuid,
    pub item_id: Uuid,
    pub title: String,
    /// The file (direct play) or the HLS master playlist.
    pub stream_url: String,
    pub position_seconds: f64,
    /// Always 0 now: HLS playlists cover the whole file.
    pub time_offset_seconds: f64,
    pub duration_seconds: Option<f64>,
    /// `direct_play`, `remux` or `transcode`.
    pub mode: String,
    pub hls: bool,
    pub reasons: Vec<TranscodeReason>,
    pub delivered: DeliveredStreams,
    pub audio_tracks: Vec<AudioTrack>,
    pub subtitle_tracks: Vec<SubtitleTrack>,
    pub selected_audio_stream_index: Option<i32>,
    pub selected_subtitle: Option<String>,
    pub subtitle_delivery: Option<SubtitleDelivery>,
}

#[derive(Deserialize)]
pub struct StartOptions {
    /// Older clients: `?transcode=true` equals `force_transcode`.
    #[serde(default)]
    transcode: bool,
}

/// Embedded text subtitles exported by every HLS run, so a client can
/// switch between them without restarting. Bounded: each costs an
/// FFmpeg output.
const MAX_EXPORTED_SUBTITLES: usize = 12;

fn embedded_text_subtitles(probe: &MediaProbe) -> Vec<i32> {
    probe
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Subtitle)
        .filter(|s| s.codec.as_deref().and_then(subtitles::kind_of_codec) == Some(SubtitleKind::Text))
        .map(|s| s.index)
        .take(MAX_EXPORTED_SUBTITLES)
        .collect()
}

fn track_name(language: Option<&str>, title: Option<&str>, forced: bool, fallback: &str) -> String {
    let mut name = match (language, title) {
        (Some(l), Some(t)) => format!("{l} - {t}"),
        (Some(l), None) => l.to_owned(),
        (None, Some(t)) => t.to_owned(),
        (None, None) => fallback.to_owned(),
    };
    if forced {
        name.push_str(" (forced)");
    }
    name
}

/// HLS subtitle renditions: embedded text streams, then external text
/// files, in that order.
fn renditions(probe: &MediaProbe, externals: &[ExternalSubtitle], selected: Option<TrackRef>) -> Vec<Rendition> {
    let embedded = embedded_text_subtitles(probe).into_iter().filter_map(|index| {
        let s = probe.stream(index)?;
        Some(Rendition {
            track: TrackRef::Embedded(index),
            name: track_name(s.language.as_deref(), s.title.as_deref(), s.is_forced, &format!("#{index}")),
            language: s.language.clone(),
            default: selected == Some(TrackRef::Embedded(index)),
            forced: s.is_forced,
        })
    });
    let external = externals.iter().enumerate().filter(|(_, e)| e.kind == SubtitleKind::Text).map(|(n, e)| Rendition {
        track: TrackRef::External(n),
        name: track_name(e.language.as_deref(), e.title.as_deref(), e.forced, &e.format),
        language: e.language.clone(),
        default: selected == Some(TrackRef::External(n)),
        forced: e.forced,
    });
    embedded.chain(external).collect()
}

fn subtitle_tracks(
    probe: &MediaProbe,
    externals: &[ExternalSubtitle],
    plan: &PlaybackPlan,
    renditions: &[Rendition],
    session_id: Uuid,
    ticket: &str,
) -> Vec<SubtitleTrack> {
    let direct = plan.method == PlayMethod::DirectPlay;
    let rendition = |track: TrackRef| (!direct).then(|| renditions.iter().position(|r| r.track == track).map(|p| p as u32)).flatten();
    let embedded = probe.streams.iter().filter(|s| s.kind == StreamKind::Subtitle).filter_map(|s| {
        let kind = subtitles::kind_of_codec(s.codec.as_deref().unwrap_or_default())?;
        Some(SubtitleTrack {
            id: TrackRef::Embedded(s.index).id(),
            language: s.language.clone(),
            title: s.title.clone(),
            codec: s.codec.clone(),
            kind,
            forced: s.is_forced,
            is_default: s.is_default,
            external: false,
            url: None,
            hls_rendition: rendition(TrackRef::Embedded(s.index)),
        })
    });
    let external = externals.iter().enumerate().map(|(n, e)| SubtitleTrack {
        id: TrackRef::External(n).id(),
        language: e.language.clone(),
        title: e.title.clone(),
        codec: Some(e.format.clone()),
        kind: e.kind,
        forced: e.forced,
        is_default: e.default,
        external: true,
        url: (direct && e.kind == SubtitleKind::Text)
            .then(|| format!("/api/playback/{session_id}/subtitles/x{n}.vtt?ticket={ticket}")),
        hls_rendition: rendition(TrackRef::External(n)),
    });
    embedded.chain(external).collect()
}

fn delivered(plan: &PlaybackPlan) -> DeliveredStreams {
    let (audio_codec, audio_channels, audio_copied, downmix) = match &plan.audio {
        None => (None, None, false, None),
        Some(audio) => match &audio.action {
            AudioAction::Copy => (Some(audio.source_codec.clone()), Some(audio.source_channels), true, None),
            AudioAction::Encode { codec, channels, downmix, .. } => (
                Some(match codec {
                    decision::AudioCodecOut::Aac => "aac".to_owned(),
                    decision::AudioCodecOut::Eac3 => "eac3".to_owned(),
                }),
                Some(*channels),
                false,
                *downmix,
            ),
        },
    };
    DeliveredStreams {
        video_codec: plan.output_video_codec.clone(),
        width: plan.output_width,
        height: plan.output_height,
        video_copied: !plan.encodes_video(),
        tone_mapped: matches!(&plan.video, VideoAction::Encode(e) if e.tonemap.is_some()),
        audio_codec,
        audio_channels,
        audio_copied,
        downmix,
    }
}

fn bad_request(code: &'static str) -> PlaybackError {
    PlaybackError(StatusCode::BAD_REQUEST, code)
}

#[utoipa::path(post, path="/api/media/{item_id}/playback", params(("item_id"=Uuid,Path), ("transcode"=Option<bool>, Query, description="Deprecated: use force_transcode")), request_body(content = Option<StartRequest>, description = "Optional; an empty body plays the default tracks"), responses((status=200,body=StartResponse),(status=400),(status=401),(status=403),(status=404),(status=503)), security(("lisek_auth"=[])))]
pub async fn start(auth: AuthUser, State(state): State<SharedState>, Path(item_id): Path<Uuid>, Query(options): Query<StartOptions>, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>, headers: HeaderMap, body: Bytes) -> Result<Json<StartResponse>, PlaybackError> {
    let request: StartRequest = if body.iter().all(u8::is_ascii_whitespace) {
        StartRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| bad_request("invalid_playback_request"))?
    };
    let client_info = lisek_telemetry::client::classify(headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()));
    let subtitle = match request.subtitle.as_deref() {
        None | Some("") => None,
        Some(id) => Some(TrackRef::parse(id).ok_or(bad_request("subtitle_track_not_found"))?),
    };
    let pool = pool(&state).await?;
    if !auth.can_see(&pool, item_id).await? {
        return Err(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"));
    }
    let source = db::source(&pool, item_id).await?.ok_or(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"))?;
    let path = checked_file(&source).await?;
    let base_settings = db::settings(&pool).await?;
    // Faza 14: the device (NVIDIA / Intel / AMD, from the pool) is chosen
    // before planning, so the plan uses that device's API and codecs. A
    // full pool plans for the CPU.
    let (settings, reservation) = match lisek_playback::gpu::reserve(&base_settings).await {
        Some((target, reservation)) => (lisek_playback::gpu::apply(&base_settings, &target), Some((target.id, reservation))),
        None => (lisek_playback::gpu::without_hardware(&base_settings), None),
    };
    let address = lisek_core::playback_settings::client_address(peer.ip(), headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()), &settings.trusted_proxies);
    let remote_limit = settings.remote_limit_kbps(address);
    let probe = match media_probe(&pool, item_id, &path, &settings).await {
        Ok(probe) => probe,
        Err(reason) => {
            tracing::warn!(%item_id, %reason, "cannot probe media");
            // Without ffprobe the file can still be offered as it is.
            if request.force_transcode || options.transcode || remote_limit.is_some() {
                return Err(PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "transcode_unavailable"));
            }
            return direct_play_without_probe(&pool, &auth, item_id, source.title, &settings, &client_info).await;
        }
    };
    let externals = {
        let file = path.clone();
        tokio::task::spawn_blocking(move || subtitles::find_external(&file)).await.unwrap_or_default()
    };
    let user_agent = headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok());
    let client = ClientCapabilities::effective(request.client.clone(), user_agent);
    let tools = transcode::tools(&settings).await;
    let playback_request = PlaybackRequest {
        audio_stream_index: request.audio_stream_index,
        subtitle,
        burn_subtitle: request.burn_subtitle,
        force_transcode: request.force_transcode || options.transcode,
        max_height: request.max_height,
        max_bitrate_kbps: request.max_bitrate_kbps,
        downmix: request.downmix,
    };
    let decide = |keyframes_available: bool| {
        decision::decide(&DecisionInput {
            probe: &probe,
            externals: &externals,
            client: &client,
            settings: &settings,
            tools: &tools,
            request: &playback_request,
            remote_limit_kbps: remote_limit,
            keyframes_available,
        })
        .map_err(|code| match code {
            "streaming_limit_too_low" | "video_stream_missing" | "resolution_unavailable" => PlaybackError(StatusCode::SERVICE_UNAVAILABLE, code),
            other => bad_request(other),
        })
    };
    let mut plan = decide(true)?;
    let mut keyframes = None;
    if plan.method == PlayMethod::Remux {
        keyframes = keyframes_for(&pool, item_id, &path, &probe).await;
        if keyframes.is_none() {
            schedule_keyframe_scan(state.clone(), item_id, path.clone(), plan.video_index, settings.clone());
            plan = decide(false)?;
        }
    }

    let saved = db::media_state(&pool, auth.user.id, item_id).await?;
    let ticket = lisek_auth::generate_session_token();
    let session_id = db::start(&pool, auth.user.id, item_id, &auth.token_hash, &lisek_auth::hash_session_token(&ticket), &session_client(&client_info)).await?;
    let duration = probe.duration_seconds;
    let position = match request.start_position_seconds {
        Some(p) if p.is_finite() && p >= 0.0 => p.min(duration.map_or(p, |d| (d - 1.0).max(0.0))),
        Some(_) => return Err(bad_request("invalid_start_position")),
        None if saved.watched => 0.0,
        None => settings.resume_state(saved.position_seconds, saved.duration_seconds).0,
    };
    let renditions = renditions(&probe, &externals, subtitle);
    let mut response = StartResponse {
        session_id,
        item_id,
        title: source.title,
        stream_url: format!("/api/playback/{session_id}/stream?ticket={ticket}"),
        position_seconds: position,
        time_offset_seconds: 0.0,
        duration_seconds: duration,
        mode: plan.method.as_str().to_owned(),
        hls: plan.method != PlayMethod::DirectPlay,
        reasons: plan.reasons.clone(),
        delivered: delivered(&plan),
        audio_tracks: probe
            .streams
            .iter()
            .filter(|s| s.kind == StreamKind::Audio)
            .map(|s| AudioTrack {
                index: s.index,
                codec: s.codec.clone(),
                language: s.language.clone(),
                title: s.title.clone(),
                channels: s.channels,
                channel_layout: s.channel_layout.clone(),
                is_default: s.is_default,
            })
            .collect(),
        subtitle_tracks: subtitle_tracks(&probe, &externals, &plan, &renditions, session_id, &ticket),
        selected_audio_stream_index: plan.audio.as_ref().map(|a| a.stream_index),
        selected_subtitle: plan.subtitle.as_ref().map(|s| s.track.id()),
        subtitle_delivery: plan.subtitle.as_ref().map(|s| s.delivery),
    };
    if plan.method == PlayMethod::DirectPlay {
        return Ok(Json(response));
    }

    // Remux or transcode: an HLS session.
    let info = transcode::source_info(&probe).map_err(|_| PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "transcode_unavailable"))?;
    if plan.encodes_video() {
        enforce_safety(&pool, session_id, &info).await?;
    }
    let origin = probe.start_time_seconds.unwrap_or(0.0);
    let segment_seconds = settings.hls_segment_seconds as f64;
    let segments = match (&plan.video, keyframes.as_deref()) {
        (VideoAction::Copy, Some(found)) => SegmentMap::from_keyframes(found, origin, info.duration, segment_seconds),
        (VideoAction::Encode(_), _) => Some(SegmentMap::fixed(info.duration, segment_seconds)),
        (VideoAction::Copy, None) => None,
    }
    .ok_or(PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "transcode_unavailable"))?;
    let root = if settings.transcoding_temp_path.trim().is_empty() {
        crate::state::config_dir().join("transcodes")
    } else {
        PathBuf::from(&settings.transcoding_temp_path).join("lisek-transcodes")
    };
    let method = plan.method;
    let encodes_video = plan.encodes_video();
    let frame_rate = probe.primary_video().and_then(|v| v.frame_rate);
    let spec = SessionSpec {
        id: session_id.to_string(),
        input: path,
        directory: root.join(session_id.to_string()),
        plan,
        segments,
        origin,
        settings: settings.clone(),
        tools,
        start_position: position,
        text_subtitles: embedded_text_subtitles(&probe),
        externals,
        renditions,
        frame_rate,
        hw_target: None,
        reservation: None,
    };
    let mut spec = spec;
    if spec.plan.uses_gpu()
        && let Some((id, reservation)) = reservation
    {
        spec.hw_target = Some(id);
        spec.reservation = Some(reservation);
    }
    if let Err(err) = session::create(spec).await {
        db::end_session(&pool, session_id).await?;
        tracing::warn!(code = err.code(), "cannot start HLS session");
        return Err(err.into());
    }
    if let Err(error) = db::set_method(&pool, session_id, method.as_str()).await {
        session::stop(&session_id.to_string()).await;
        return Err(error.into());
    }
    response.stream_url = format!("/api/playback/{session_id}/hls/master.m3u8?ticket={ticket}");

    // Ends the session with the login it belongs to, and re-checks the
    // transcode limits while it runs.
    let guard_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let Some(pool) = guard_state.pool() else { break };
            if !db::session_alive(&pool, session_id).await.unwrap_or(false) {
                break;
            }
            if encodes_video && enforce_safety(&pool, session_id, &info).await.is_err() {
                break;
            }
            if !session::exists(&session_id.to_string()) {
                break;
            }
        }
        session::stop(&session_id.to_string()).await;
    });
    Ok(Json(response))
}

/// The session row's view of the client (Faza 13 statistics).
fn session_client(info: &lisek_telemetry::client::ClientInfo) -> db::SessionClient<'_> {
    db::SessionClient {
        user_agent: info.user_agent.as_deref(),
        platform: info.platform.as_str(),
        device_type: info.device_type.as_str(),
        browser: info.browser,
    }
}

/// Direct play of a file nothing is known about (ffprobe missing or the
/// file unreadable to it): the client gets the file as it is.
async fn direct_play_without_probe(
    pool: &lisek_db::PgPool,
    auth: &AuthUser,
    item_id: Uuid,
    title: String,
    settings: &PlaybackSettings,
    client: &lisek_telemetry::client::ClientInfo,
) -> Result<Json<StartResponse>, PlaybackError> {
    let saved = db::media_state(pool, auth.user.id, item_id).await?;
    let ticket = lisek_auth::generate_session_token();
    let session_id = db::start(pool, auth.user.id, item_id, &auth.token_hash, &lisek_auth::hash_session_token(&ticket), &session_client(client)).await?;
    let position = if saved.watched { 0.0 } else { settings.resume_state(saved.position_seconds, saved.duration_seconds).0 };
    Ok(Json(StartResponse {
        session_id,
        item_id,
        title,
        stream_url: format!("/api/playback/{session_id}/stream?ticket={ticket}"),
        position_seconds: position,
        time_offset_seconds: 0.0,
        duration_seconds: None,
        mode: PlayMethod::DirectPlay.as_str().to_owned(),
        hls: false,
        reasons: vec![],
        delivered: DeliveredStreams {
            video_codec: String::new(),
            width: 0,
            height: 0,
            video_copied: true,
            tone_mapped: false,
            audio_codec: None,
            audio_channels: None,
            audio_copied: true,
            downmix: None,
        },
        audio_tracks: vec![],
        subtitle_tracks: vec![],
        selected_audio_stream_index: None,
        selected_subtitle: None,
        subtitle_delivery: None,
    }))
}

#[derive(Deserialize)]
pub struct TicketQuery {
    ticket: String,
}

fn valid_ticket(ticket: &str) -> bool {
    !ticket.is_empty() && ticket.len() <= 128 && ticket.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

async fn ticket_source(state: &SharedState, id: Uuid, ticket: &str) -> Result<db::Source, PlaybackError> {
    if !valid_ticket(ticket) {
        return Err(PlaybackError(StatusCode::UNAUTHORIZED, "invalid_playback_ticket"));
    }
    db::ticket_source(&pool(state).await?, id, &lisek_auth::hash_session_token(ticket))
        .await?
        .ok_or(PlaybackError(StatusCode::UNAUTHORIZED, "invalid_playback_ticket"))
}

fn text_response(content_type: &'static str, body: String) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "private, no-store")], body).into_response()
}

async fn file_response(path: &FsPath, content_type: &'static str) -> Result<Response, PlaybackError> {
    let file = tokio::fs::File::open(path).await.map_err(|_| PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "segment_not_ready"))?;
    let length = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, length.to_string())
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from_stream(ReaderStream::new(file)))
        .expect("static response headers"))
}

#[utoipa::path(get,path="/api/playback/{session_id}/hls/{file}",params(("session_id"=Uuid,Path),("file"=String,Path,description="master.m3u8, video.m3u8, init.mp4, seg-<n>.m4s, subs-<track>.m3u8, subs-<track>-<n>.vtt"),("ticket"=String,Query)),responses((status=200,description="HLS playlist, segment or WebVTT"),(status=401),(status=404),(status=503)))]
pub async fn hls(State(state): State<SharedState>, Path((id, file)): Path<(Uuid, String)>, Query(query): Query<TicketQuery>) -> Result<Response, PlaybackError> {
    let file = HlsFile::parse(&file).ok_or(PlaybackError(StatusCode::NOT_FOUND, "segment_not_found"))?;
    ticket_source(&state, id, &query.ticket).await?;
    let session = session::get(&id.to_string()).ok_or(PlaybackError(StatusCode::NOT_FOUND, "playback_not_found"))?;
    let suffix = format!("?ticket={}", query.ticket);
    let content_type = file.content_type();
    match file {
        HlsFile::Master => Ok(text_response(content_type, session.master_playlist(&suffix).await?)),
        HlsFile::Video => {
            session.touch();
            Ok(text_response(content_type, session.video_playlist(&suffix)))
        }
        HlsFile::SubtitlePlaylist(track) => Ok(text_response(content_type, session.subtitle_playlist(track, &suffix)?)),
        HlsFile::SubtitleSegment(track, index) => Ok(text_response(content_type, session.subtitle_segment(track, index).await?)),
        HlsFile::Init => file_response(&session.init_segment().await?, content_type).await,
        HlsFile::Segment(index) => file_response(&session.media_segment(index).await?, content_type).await,
    }
}

#[utoipa::path(get,path="/api/playback/{session_id}/subtitles/{file}",params(("session_id"=Uuid,Path),("file"=String,Path,description="x<n>.vtt"),("ticket"=String,Query)),responses((status=200,description="WebVTT"),(status=401),(status=404),(status=503)))]
pub async fn subtitles(State(state): State<SharedState>, Path((id, file)): Path<(Uuid, String)>, Query(query): Query<TicketQuery>) -> Result<Response, PlaybackError> {
    let not_found = || PlaybackError(StatusCode::NOT_FOUND, "subtitle_track_not_found");
    let Some(TrackRef::External(n)) = file.strip_suffix(".vtt").and_then(TrackRef::parse) else {
        return Err(not_found());
    };
    let source = ticket_source(&state, id, &query.ticket).await?;
    let path = checked_file(&source).await?;
    let externals = tokio::task::spawn_blocking(move || subtitles::find_external(&path)).await.unwrap_or_default();
    let external = externals.get(n).ok_or_else(not_found)?;
    let settings = db::settings(&pool(&state).await?).await?;
    let vtt = session::convert_external(external, &settings, None).await.map_err(|reason| {
        tracing::warn!(%reason, "subtitle conversion failed");
        PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "subtitle_conversion_failed")
    })?;
    Ok(text_response("text/vtt; charset=utf-8", vtt))
}

#[utoipa::path(get,path="/api/admin/playback/safety",responses((status=200,body=db::SafetyConfig),(status=403)),security(("lisek_auth"=[])))]
pub async fn safety(_admin: AdminUser, State(state): State<SharedState>) -> Result<Json<db::SafetyConfig>, PlaybackError> {
    Ok(Json(db::safety(&pool(&state).await?).await?))
}
#[utoipa::path(put,path="/api/admin/playback/safety",request_body=db::SafetyConfig,responses((status=204),(status=400),(status=403)),security(("lisek_auth"=[])))]
pub async fn set_safety(_admin: AdminUser, State(state): State<SharedState>, Json(req): Json<db::SafetyConfig>) -> Result<StatusCode, PlaybackError> {
    if !(1..=16384).contains(&req.max_width) || !(1..=16384).contains(&req.max_height) || req.max_bitrate_kbps.is_some_and(|b| b <= 0) {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_safety_limits"));
    }
    db::set_safety(&pool(&state).await?, &req).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path="/api/playback/{session_id}/stream", params(("session_id"=Uuid,Path),("ticket"=String,Query)), responses((status=200,description="Video bytes"),(status=206,description="Partial video bytes"),(status=401),(status=416)))]
pub async fn stream(State(state): State<SharedState>, Path(id): Path<Uuid>, Query(query): Query<TicketQuery>, headers: HeaderMap, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>) -> Result<Response, PlaybackError> {
    let source = ticket_source(&state, id, &query.ticket).await?;
    let pool = pool(&state).await?;
    let path = checked_file(&source).await?;
    let settings = db::settings(&pool).await?;
    let address = lisek_core::playback_settings::client_address(peer.ip(), headers.get("x-forwarded-for").and_then(|h| h.to_str().ok()), &settings.trusted_proxies);
    if let Some(limit) = settings.remote_limit_kbps(address) {
        let probe = media_probe(&pool, source.id, &path, &settings).await.map_err(|_| PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "transcode_unavailable"))?;
        let bitrate = probe.bitrate_kbps.and_then(|b| u32::try_from(b).ok()).unwrap_or(0);
        if bitrate == 0 || bitrate > limit {
            return Err(PlaybackError(StatusCode::FORBIDDEN, "remote_transcode_required"));
        }
    }
    let mut file = tokio::fs::File::open(&path).await.map_err(|_| PlaybackError(StatusCode::NOT_FOUND, "media_file_unavailable"))?;
    let length = file.metadata().await.map_err(|_| PlaybackError(StatusCode::INTERNAL_SERVER_ERROR, "media_read_failed"))?.len();
    let range = match headers.get(header::RANGE) {
        Some(value) => match value.to_str().ok().and_then(|value| lisek_playback::parse_range(value, length)) {
            Some(range) => Some(range),
            None => return Ok((StatusCode::RANGE_NOT_SATISFIABLE, [(header::CONTENT_RANGE, format!("bytes */{length}"))]).into_response()),
        },
        None => None,
    };
    let (start, count) = range.map(|range| (range.start, range.end - range.start + 1)).unwrap_or((0, length));
    file.seek(std::io::SeekFrom::Start(start)).await.map_err(|_| PlaybackError(StatusCode::INTERNAL_SERVER_ERROR, "media_read_failed"))?;
    let mut builder = Response::builder()
        .status(if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, lisek_playback::content_type(&path))
        .header(header::CONTENT_LENGTH, count.to_string())
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff");
    if let Some(range) = range {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {}-{}/{length}", range.start, range.end));
    }
    Ok(builder.body(Body::from_stream(ReaderStream::new(file.take(count)))).expect("static response headers"))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct FlagsRequest {
    watched: Option<bool>,
    favorite: Option<bool>,
}
#[utoipa::path(get,path="/api/media/{item_id}/state",params(("item_id"=Uuid,Path)),responses((status=200,body=Value),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn media_state(auth: AuthUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<Json<Value>, PlaybackError> {
    let pool = pool(&state).await?;
    if !auth.can_see(&pool, id).await? || db::source(&pool, id).await?.is_none() {
        return Err(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"));
    }
    Ok(Json(json!(db::media_state(&pool, auth.user.id, id).await?)))
}
#[utoipa::path(put,path="/api/media/{item_id}/state",params(("item_id"=Uuid,Path)),request_body=FlagsRequest,responses((status=204),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn set_flags(auth: AuthUser, State(state): State<SharedState>, Path(id): Path<Uuid>, Json(req): Json<FlagsRequest>) -> Result<StatusCode, PlaybackError> {
    let pool = pool(&state).await?;
    if !auth.can_see(&pool, id).await? || db::source(&pool, id).await?.is_none() {
        return Err(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"));
    }
    db::set_flags(&pool, auth.user.id, id, req.watched, req.favorite).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(get,path="/api/me/list",responses((status=200,body=Value),(status=401)),security(("lisek_auth"=[])))]
pub async fn my_list(auth: AuthUser, State(state): State<SharedState>) -> Result<Json<Value>, PlaybackError> {
    Ok(Json(json!(db::my_list(&pool(&state).await?, auth.user.id, &auth.access).await?)))
}
#[utoipa::path(get,path="/api/admin/stats/users",responses((status=200,body=Value),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn stats(_admin: AdminUser, State(state): State<SharedState>) -> Result<Json<Value>, PlaybackError> {
    Ok(Json(json!(db::stats(&pool(&state).await?).await?)))
}

#[utoipa::path(get,path="/api/admin/playback/settings",responses((status=200,body=lisek_core::playback_settings::PlaybackSettings),(status=403)),security(("lisek_auth"=[])))]
pub async fn settings(_admin: AdminUser, State(state): State<SharedState>) -> Result<Json<PlaybackSettings>, PlaybackError> {
    Ok(Json(db::settings(&pool(&state).await?).await?))
}
#[utoipa::path(put,path="/api/admin/playback/settings",request_body=lisek_core::playback_settings::PlaybackSettings,responses((status=204),(status=400),(status=403)),security(("lisek_auth"=[])))]
pub async fn set_settings(_admin: AdminUser, State(state): State<SharedState>, Json(req): Json<PlaybackSettings>) -> Result<StatusCode, PlaybackError> {
    if !req.validate() || (!req.transcoding_temp_path.is_empty() && !std::path::Path::new(&req.transcoding_temp_path).is_absolute()) {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_playback_settings"));
    }
    db::set_settings(&pool(&state).await?, &req).await?;
    transcode::forget_tools();
    Ok(StatusCode::NO_CONTENT)
}
#[utoipa::path(post,path="/api/admin/playback/capabilities",request_body=lisek_core::playback_settings::PlaybackSettings,responses((status=200,body=transcode::Capabilities),(status=400),(status=403),(status=503)),security(("lisek_auth"=[])))]
pub async fn capabilities(_admin: AdminUser, Json(req): Json<PlaybackSettings>) -> Result<Json<transcode::Capabilities>, PlaybackError> {
    if !req.validate() {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_playback_settings"));
    }
    let caps = transcode::capabilities(&req).await.map_err(|_| PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "ffmpeg_unavailable"))?;
    Ok(Json(caps))
}

#[utoipa::path(delete,path="/api/playback/{session_id}",params(("session_id"=Uuid,Path)),responses((status=204),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn stop(auth: AuthUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<StatusCode, PlaybackError> {
    if !db::end_owned_session(&pool(&state).await?, auth.user.id, id).await? {
        return Err(PlaybackError(StatusCode::NOT_FOUND, "playback_not_found"));
    }
    session::stop(&id.to_string()).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tickets_are_checked_before_reaching_playlists() {
        assert!(valid_ticket("AbC-12_x"));
        for bad in ["", "a b", "x\ny", "a\"b", &"a".repeat(129)] {
            assert!(!valid_ticket(bad), "{bad:?}");
        }
    }

    #[test]
    fn start_request_accepts_an_empty_object_and_rejects_unknown_fields() {
        let request: StartRequest = serde_json::from_str("{}").unwrap();
        assert!(request.client.is_none() && request.downmix == DownmixPreference::Auto);
        let request: StartRequest = serde_json::from_str(r#"{"audio_stream_index":2,"subtitle":"x0","downmix":"night","client":{"video_codecs":["h264"],"hls":"native"}}"#).unwrap();
        assert_eq!(request.audio_stream_index, Some(2));
        assert_eq!(request.downmix, DownmixPreference::Night);
        assert!(serde_json::from_str::<StartRequest>(r#"{"bogus":1}"#).is_err());
    }
}
