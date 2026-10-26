//! The detail page's extras (Faza 9): original title, tagline, rating,
//! runtime, genres, tags, studios, external links, directors and writers,
//! the streams to pick before playback, and chapters (scenes) with their
//! thumbnails.
//!
//! Everything is read from what scans already stored — provider data in
//! `media_items.metadata`, the ffprobe result in `media_streams`, chapters
//! in `media_chapters` — so the page never waits for ffprobe or a
//! provider. Only a chapter thumbnail is made on demand: the first request
//! cuts one frame with FFmpeg (two at a time server-wide) and every later
//! one reads the cached JPEG.

use std::path::{Path as FsPath, PathBuf};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use lisek_core::media_probe::{HdrFormat, MediaChapter, MediaProbe, MediaStream, StreamKind};
use lisek_db::streams;
use lisek_playback::subtitles::{self, SubtitleKind};
use lisek_playback::{chapters as thumbnails, transcode};

use crate::auth::AuthUser;
use crate::playback::{PlaybackError, checked_file, pool};
use crate::state::{SharedState, config_dir};

/// Files without chapters get one scene every this many seconds.
pub const GENERATED_SCENE_SECONDS: f64 = 600.0;
/// At most this many generated scenes (a 9-hour file stops here).
const MAX_GENERATED_SCENES: usize = 54;
/// Tags shown on the page.
const MAX_TAGS: usize = 20;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct NamedLink {
    pub name: String,
    pub url: String,
}

/// A director or writer, linking to their page.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CrewMember {
    pub person_id: Uuid,
    pub name: String,
}

/// The picture, as one line ("4K HEVC HDR").
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VideoSummary {
    pub label: String,
    /// `"4K"`, `"1080p"`, `"720p"`, `"SD"`.
    pub resolution: Option<String>,
    pub codec: Option<String>,
    /// `"HDR"`, `"HLG"`, `"Dolby Vision"`.
    pub hdr: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct AudioOption {
    /// ffprobe stream index: `audio_stream_index` of the playback request.
    pub index: i32,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    pub channels: Option<i32>,
    pub is_default: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SubtitleOption {
    /// `subtitle` of the playback request: `s<index>` embedded, `x<n>` a
    /// file next to the video.
    pub id: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub codec: Option<String>,
    /// `"text"` or `"image"` (image subtitles are burnt into the picture).
    pub kind: String,
    pub forced: bool,
    pub is_default: bool,
    pub external: bool,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ChapterResponse {
    pub index: usize,
    /// The file's own chapter name; `None` for numbered or generated ones.
    pub title: Option<String>,
    pub start_seconds: f64,
    /// `/api/media/{id}/chapters/{index}/image?v=...` (`?token=` works).
    pub image_url: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ItemDetailsResponse {
    /// Shown under the title when it differs from it.
    pub original_title: Option<String>,
    pub tagline: Option<String>,
    /// Length of the file (or of an episode, from the provider).
    pub runtime_minutes: Option<u32>,
    /// TMDB user score, 0-10.
    pub rating: Option<f64>,
    pub genres: Vec<String>,
    pub tags: Vec<String>,
    pub studios: Vec<String>,
    /// IMDb, TheTVDB, TMDB, in that order, when the item has those ids.
    pub external_links: Vec<NamedLink>,
    pub directors: Vec<CrewMember>,
    pub writers: Vec<CrewMember>,
    /// Streams of the file (films; a series card's representative episode
    /// has its own). `None` until the file was probed.
    pub video: Option<VideoSummary>,
    pub audio_tracks: Vec<AudioOption>,
    pub subtitle_tracks: Vec<SubtitleOption>,
    /// The audio stream played when none is picked.
    pub default_audio_index: Option<i32>,
    pub chapters: Vec<ChapterResponse>,
    /// `true` when the file has no chapters and `chapters` are evenly
    /// spaced scenes instead.
    pub chapters_generated: bool,
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn list(value: &Value, key: &str, limit: usize) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .take(limit)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// A positive integer id stored as a number or a numeric string.
fn numeric_id(value: &Value, key: &str) -> Option<u64> {
    match value.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|id| *id > 0)
}

/// IMDb, TheTVDB and TMDB pages of an item, from the ids its provider
/// data carries.
fn external_links(provider_data: &Value, is_series: bool) -> Vec<NamedLink> {
    let mut links = Vec::new();
    if let Some(imdb) = text(provider_data, "imdb_id").filter(|id| {
        id.starts_with("tt") && id[2..].chars().all(|c| c.is_ascii_digit()) && id.len() > 2
    }) {
        links.push(NamedLink {
            name: "IMDb".into(),
            url: format!("https://www.imdb.com/title/{imdb}/"),
        });
    }
    if let Some(tvdb) = numeric_id(provider_data, "tvdb_id") {
        let kind = if is_series { "series" } else { "movie" };
        links.push(NamedLink {
            name: "TheTVDB".into(),
            url: format!("https://thetvdb.com/dereferrer/{kind}/{tvdb}"),
        });
    }
    if let Some(tmdb) = numeric_id(provider_data, "tmdb_id") {
        let kind = if is_series { "tv" } else { "movie" };
        links.push(NamedLink {
            name: "TMDB".into(),
            url: format!("https://www.themoviedb.org/{kind}/{tmdb}"),
        });
    }
    links
}

/// Resolution class of a picture, by its larger side (a 1920x800 scope
/// film is still 1080p).
fn resolution(width: Option<i32>, height: Option<i32>) -> Option<&'static str> {
    let (w, h) = (width.unwrap_or(0), height.unwrap_or(0));
    if w <= 0 && h <= 0 {
        return None;
    }
    Some(if w >= 3200 || h >= 2000 {
        "4K"
    } else if w >= 2200 || h >= 1300 {
        "1440p"
    } else if w >= 1700 || h >= 1000 {
        "1080p"
    } else if w >= 1100 || h >= 700 {
        "720p"
    } else if h >= 560 {
        "576p"
    } else {
        "SD"
    })
}

fn codec_name(codec: &str) -> String {
    match codec {
        "hevc" | "h265" => "HEVC".into(),
        "h264" | "avc" => "H264".into(),
        "av1" => "AV1".into(),
        "vp9" => "VP9".into(),
        "vp8" => "VP8".into(),
        "mpeg2video" => "MPEG-2".into(),
        "mpeg4" => "MPEG-4".into(),
        "vc1" => "VC-1".into(),
        other => other.to_ascii_uppercase(),
    }
}

fn video_summary(video: &MediaStream) -> VideoSummary {
    let resolution = resolution(video.width, video.height).map(str::to_owned);
    let codec = video.codec.as_deref().map(codec_name);
    let hdr = video.hdr_format().map(|f| {
        match f {
            HdrFormat::Hdr10 => "HDR",
            HdrFormat::Hlg => "HLG",
            HdrFormat::DolbyVision { .. } => "Dolby Vision",
        }
        .to_owned()
    });
    let label = [&resolution, &codec, &hdr]
        .iter()
        .filter_map(|p| p.as_deref())
        .collect::<Vec<_>>()
        .join(" ");
    VideoSummary {
        label,
        resolution,
        codec,
        hdr,
    }
}

fn audio_options(probe: &MediaProbe) -> Vec<AudioOption> {
    probe
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Audio)
        .map(|s| AudioOption {
            index: s.index,
            language: s.language.clone(),
            title: s.title.clone(),
            codec: s.codec.clone(),
            channels: s.channels,
            is_default: s.is_default,
        })
        .collect()
}

fn kind_name(kind: SubtitleKind) -> String {
    match kind {
        SubtitleKind::Text => "text",
        SubtitleKind::Image => "image",
    }
    .into()
}

fn subtitle_options(
    probe: &MediaProbe,
    externals: &[subtitles::ExternalSubtitle],
) -> Vec<SubtitleOption> {
    let embedded = probe
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Subtitle)
        .filter_map(|s| {
            let kind = subtitles::kind_of_codec(s.codec.as_deref().unwrap_or_default())?;
            Some(SubtitleOption {
                id: subtitles::TrackRef::Embedded(s.index).id(),
                language: s.language.clone(),
                title: s.title.clone(),
                codec: s.codec.clone(),
                kind: kind_name(kind),
                forced: s.is_forced,
                is_default: s.is_default,
                external: false,
            })
        });
    let external = externals.iter().enumerate().map(|(n, e)| SubtitleOption {
        id: subtitles::TrackRef::External(n).id(),
        language: e.language.clone(),
        title: e.title.clone(),
        codec: Some(e.format.clone()),
        kind: kind_name(e.kind),
        forced: e.forced,
        is_default: e.default,
        external: true,
    });
    embedded.chain(external).collect()
}

/// The scenes the page shows: the file's chapters, or — for a file without
/// any — one every [`GENERATED_SCENE_SECONDS`]. `bool`: generated.
fn scenes(
    stored: Vec<MediaChapter>,
    duration: Option<f64>,
    probed: bool,
) -> (Vec<MediaChapter>, bool) {
    if !stored.is_empty() {
        return (stored, false);
    }
    let Some(duration) =
        duration.filter(|d| probed && d.is_finite() && *d >= GENERATED_SCENE_SECONDS)
    else {
        return (Vec::new(), false);
    };
    let count = ((duration / GENERATED_SCENE_SECONDS).ceil() as usize).min(MAX_GENERATED_SCENES);
    let generated = (0..count)
        .map(|n| {
            let start = n as f64 * GENERATED_SCENE_SECONDS;
            MediaChapter {
                start_seconds: start,
                end_seconds: Some((start + GENERATED_SCENE_SECONDS).min(duration)),
                title: None,
            }
        })
        .collect();
    (generated, true)
}

async fn item_scenes(
    pool: &lisek_db::PgPool,
    item_id: Uuid,
) -> Result<(Vec<MediaChapter>, bool), PlaybackError> {
    let (_, duration, probed) = streams::details_source(pool, item_id)
        .await?
        .ok_or(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"))?;
    Ok(scenes(
        streams::chapters(pool, item_id).await?,
        duration,
        probed,
    ))
}

fn start_millis(chapter: &MediaChapter) -> u64 {
    (chapter.start_seconds.max(0.0) * 1000.0).round() as u64
}

#[utoipa::path(get,path="/api/media/{item_id}/details",tag="media",params(("item_id"=Uuid,Path)),responses((status=200,body=ItemDetailsResponse),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn item_details(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<ItemDetailsResponse>, PlaybackError> {
    let pool = pool(&state).await?;
    let not_found = || PlaybackError(StatusCode::NOT_FOUND, "media_not_found");
    let item = lisek_db::get_media_item(&pool, item_id, &auth.access)
        .await?
        .ok_or_else(not_found)?;
    let is_series = item.item_type == "series";
    let (metadata, duration, probed) = streams::details_source(&pool, item_id)
        .await?
        .ok_or_else(not_found)?;
    let provider = metadata
        .get("provider_data")
        .cloned()
        .unwrap_or(Value::Null);

    // An administrator's own original title wins over the provider's; one
    // equal to the title (the scan's default) is no original title.
    let differs = |o: &String| !o.eq_ignore_ascii_case(item.title.trim());
    let original_title = text(&metadata, "original_title")
        .filter(differs)
        .or_else(|| text(&provider, "original_title").filter(differs));
    let provider_runtime = provider
        .get("runtime_minutes")
        .and_then(Value::as_u64)
        .and_then(|m| u32::try_from(m).ok());
    // A film's own length is exact; a series card stands for the whole show.
    let runtime_minutes = if is_series {
        provider_runtime
    } else {
        duration
            .filter(|d| d.is_finite() && *d >= 60.0)
            .map(|d| (d / 60.0).round() as u32)
            .or(provider_runtime)
    };

    let credits = lisek_db::people::item_credits(&pool, item_id, 60).await?;
    let crew = |role: &str| {
        credits
            .iter()
            .filter(|c| c.role == role)
            .map(|c| CrewMember {
                person_id: c.person_id,
                name: c.name.clone(),
            })
            .collect::<Vec<_>>()
    };

    let probe = streams::stored_probe(&pool, item_id)
        .await?
        .map(|(probe, _)| probe);
    let externals = match (&probe, lisek_db::playback::source(&pool, item_id).await?) {
        (Some(_), Some(source)) => match checked_file(&source).await {
            Ok(path) => tokio::task::spawn_blocking(move || subtitles::find_external(&path))
                .await
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        },
        _ => Vec::new(),
    };

    let (chapters, chapters_generated) =
        scenes(streams::chapters(&pool, item_id).await?, duration, probed);

    Ok(Json(ItemDetailsResponse {
        original_title,
        tagline: text(&provider, "tagline"),
        runtime_minutes,
        rating: provider
            .get("rating")
            .and_then(Value::as_f64)
            .filter(|r| r.is_finite() && *r > 0.0 && *r <= 10.0),
        genres: list(&provider, "genres", 10),
        tags: list(&provider, "tags", MAX_TAGS),
        studios: list(&provider, "studios", 8),
        external_links: external_links(&provider, is_series),
        directors: crew("Director"),
        writers: crew("Writer"),
        video: probe
            .as_ref()
            .and_then(|p| p.primary_video())
            .map(video_summary),
        audio_tracks: probe.as_ref().map(audio_options).unwrap_or_default(),
        subtitle_tracks: probe
            .as_ref()
            .map(|p| subtitle_options(p, &externals))
            .unwrap_or_default(),
        default_audio_index: probe
            .as_ref()
            .and_then(|p| p.primary_audio())
            .map(|s| s.index),
        chapters: chapters
            .iter()
            .enumerate()
            .map(|(index, c)| ChapterResponse {
                index,
                title: c.title.clone(),
                start_seconds: c.start_seconds,
                image_url: format!(
                    "/api/media/{item_id}/chapters/{index}/image?v={}",
                    start_millis(c)
                ),
            })
            .collect(),
        chapters_generated,
    }))
}

/// `config/chapters/<item>/`.
fn image_dir(item_id: Uuid) -> PathBuf {
    config_dir().join("chapters").join(item_id.to_string())
}

/// Named by the moment shown, so changed chapters never serve an old
/// picture.
fn image_path(item_id: Uuid, chapter: &MediaChapter) -> PathBuf {
    image_dir(item_id).join(format!("{}.jpg", start_millis(chapter)))
}

fn jpeg(bytes: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            // The URL carries the chapter's start: safe to keep for good.
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response()
}

/// FFmpeg runs for at most two thumbnails at a time, server-wide.
pub(crate) static GENERATING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

fn unavailable() -> PlaybackError {
    PlaybackError(StatusCode::NOT_FOUND, "chapter_image_unavailable")
}

/// A chapter's thumbnail (JPEG), made on the first request.
#[utoipa::path(get,path="/api/media/{item_id}/chapters/{index}/image",tag="media",params(("item_id"=Uuid,Path),("index"=usize,Path)),responses((status=200,description="JPEG"),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn chapter_image(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path((item_id, index)): Path<(Uuid, usize)>,
) -> Result<Response, PlaybackError> {
    let pool = pool(&state).await?;
    if !auth.can_see(&pool, item_id).await? {
        return Err(unavailable());
    }
    let (chapters, _) = item_scenes(&pool, item_id).await?;
    let chapter = chapters.get(index).ok_or_else(unavailable)?;
    let path = image_path(item_id, chapter);
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok(jpeg(bytes));
    }

    let _permit = GENERATING.acquire().await.map_err(|_| unavailable())?;
    // Another request may have made it while this one waited.
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok(jpeg(bytes));
    }
    let source = lisek_db::playback::source(&pool, item_id)
        .await?
        .ok_or_else(unavailable)?;
    let input = checked_file(&source).await?;
    let probe = streams::stored_probe(&pool, item_id)
        .await?
        .map(|(probe, _)| probe);
    let video = probe
        .as_ref()
        .and_then(|p| p.primary_video())
        .ok_or_else(unavailable)?;
    let settings = lisek_db::playback::settings(&pool).await?;
    let tools = transcode::tools(&settings).await;
    let at = thumbnails::thumbnail_time(chapter.start_seconds, chapter.end_seconds);
    let started = std::time::Instant::now();
    if let Err(err) = thumbnails::generate(
        &settings,
        &tools,
        &input,
        video.index,
        at,
        video.color_transfer.as_deref(),
        &path,
    )
    .await
    {
        tracing::warn!(%item_id, index, %err, "chapter thumbnail failed");
        return Err(unavailable());
    }
    tracing::debug!(%item_id, index, at, elapsed_ms = started.elapsed().as_millis() as u64, "chapter thumbnail made");
    let bytes = tokio::fs::read(&path).await.map_err(|_| unavailable())?;
    Ok(jpeg(bytes))
}

/// Removes chapter thumbnails and episode stills of items that no longer
/// exist (after scans).
pub async fn sweep_chapter_images(pool: &lisek_db::PgPool) {
    sweep_in(pool, &config_dir().join("chapters")).await;
    // Episode stills and frames (Faza 10) follow the same rule.
    sweep_in(pool, &config_dir().join("metadata").join("episodes")).await;
}

async fn sweep_in(pool: &lisek_db::PgPool, root: &FsPath) {
    let Ok(mut entries) = tokio::fs::read_dir(root).await else {
        return;
    };
    let mut folders = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        folders.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
        ));
    }
    let ids: Vec<Uuid> = folders
        .iter()
        .filter_map(|(name, _)| name.parse().ok())
        .collect();
    let Ok(existing) = streams::existing_items(pool, &ids).await else {
        return;
    };
    for (name, path) in folders {
        let keep = name.parse::<Uuid>().is_ok_and(|id| existing.contains(&id));
        if !keep {
            let _ = tokio::fs::remove_dir_all(path).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_come_from_the_provider_ids() {
        let data = serde_json::json!({"imdb_id": "tt0322259", "tmdb_id": 584, "tvdb_id": "81189"});
        let links = external_links(&data, false);
        assert_eq!(
            links
                .iter()
                .map(|l| (l.name.as_str(), l.url.as_str()))
                .collect::<Vec<_>>(),
            [
                ("IMDb", "https://www.imdb.com/title/tt0322259/"),
                ("TheTVDB", "https://thetvdb.com/dereferrer/movie/81189"),
                ("TMDB", "https://www.themoviedb.org/movie/584"),
            ]
        );
        assert_eq!(
            external_links(&data, true)[2].url,
            "https://www.themoviedb.org/tv/584"
        );
        let bad = serde_json::json!({"imdb_id": "nm1", "tmdb_id": 0, "tvdb_id": "x"});
        assert!(external_links(&bad, false).is_empty());
    }

    #[test]
    fn the_picture_reads_as_one_line() {
        let mut video = MediaStream {
            index: 0,
            kind: StreamKind::Video,
            codec: Some("hevc".into()),
            profile: None,
            language: None,
            title: None,
            is_default: true,
            is_forced: false,
            width: Some(3840),
            height: Some(1600),
            bit_depth: Some(10),
            pixel_format: None,
            color_transfer: Some("smpte2084".into()),
            color_primaries: None,
            color_space: None,
            field_order: None,
            frame_rate: None,
            channels: None,
            channel_layout: None,
            sample_rate: None,
            bitrate_kbps: None,
            dovi_profile: None,
        };
        assert_eq!(video_summary(&video).label, "4K HEVC HDR");
        video.dovi_profile = Some(8);
        assert_eq!(video_summary(&video).hdr.as_deref(), Some("Dolby Vision"));
        video.width = Some(1920);
        video.height = Some(800);
        video.color_transfer = None;
        video.dovi_profile = None;
        video.codec = Some("h264".into());
        assert_eq!(video_summary(&video).label, "1080p H264");
        assert_eq!(resolution(Some(720), Some(576)), Some("576p"));
        assert_eq!(resolution(Some(640), Some(360)), Some("SD"));
        assert_eq!(resolution(None, None), None);
    }

    #[test]
    fn files_without_chapters_get_evenly_spaced_scenes() {
        let (none, generated) = scenes(Vec::new(), Some(300.0), true);
        assert!(none.is_empty() && !generated, "too short for scenes");
        let (scenes_of_film, generated) = scenes(Vec::new(), Some(6480.0), true);
        assert!(generated);
        assert_eq!(scenes_of_film.len(), 11);
        assert_eq!(scenes_of_film[10].start_seconds, 6000.0);
        assert_eq!(scenes_of_film[10].end_seconds, Some(6480.0));
        assert!(
            scenes(Vec::new(), Some(6480.0), false).0.is_empty(),
            "not probed yet"
        );
        assert_eq!(
            scenes(Vec::new(), Some(1e9), true).0.len(),
            MAX_GENERATED_SCENES
        );
        let own = vec![MediaChapter {
            start_seconds: 0.0,
            end_seconds: None,
            title: Some("A".into()),
        }];
        assert_eq!(scenes(own.clone(), Some(6480.0), true), (own, false));
    }

    #[test]
    fn lists_skip_blanks_and_respect_the_limit() {
        let data =
            serde_json::json!({"tags": ["car race", " ", 3, "sequel", "x"], "tagline": "  "});
        assert_eq!(list(&data, "tags", 2), ["car race", "sequel"]);
        assert_eq!(text(&data, "tagline"), None);
        assert!(list(&data, "missing", 5).is_empty());
    }
}
