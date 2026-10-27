//! FFmpeg and ffprobe binaries: locating them, probing files, and finding
//! out what the installed build can do. The HLS engine itself lives in
//! [`crate::session`].
use lisek_core::media_probe::{MediaChapter, MediaProbe, MediaStream, StreamKind};
use lisek_core::playback_settings::PlaybackSettings;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
    pub bitrate_kbps: u32,
    pub duration: f64,
    pub codec: String,
    pub pixel_format: String,
    pub hdr: bool,
    pub interlaced: bool,
    pub audio_channels: u32,
    /// ffprobe index of the stream to encode; skips cover-art "video"
    /// streams that `-map 0:v:0` would otherwise pick.
    pub video_index: Option<i32>,
}
/// Raw `ffprobe -show_streams -show_format -of json` output. Every field is
/// optional: ffprobe omits keys freely depending on the codec.
#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
    #[serde(default)]
    chapters: Vec<Chapter>,
}
#[derive(Deserialize)]
struct Chapter {
    start_time: Option<String>,
    end_time: Option<String>,
    #[serde(default)]
    tags: HashMap<String, String>,
}
#[derive(Deserialize)]
struct Stream {
    index: Option<i32>,
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
    pix_fmt: Option<String>,
    bits_per_raw_sample: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    color_space: Option<String>,
    field_order: Option<String>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    channels: Option<i32>,
    channel_layout: Option<String>,
    sample_rate: Option<String>,
    bit_rate: Option<String>,
    #[serde(default)]
    disposition: HashMap<String, i64>,
    #[serde(default)]
    tags: HashMap<String, String>,
    #[serde(default)]
    side_data_list: Vec<SideData>,
}
#[derive(Deserialize)]
struct SideData {
    side_data_type: Option<String>,
    dv_profile: Option<i32>,
}
#[derive(Deserialize, Default)]
struct Format {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
    start_time: Option<String>,
}

pub fn binary_path(name: &str, config: &PlaybackSettings) -> PathBuf {
    // An administrator's saved path takes precedence; environment values configure fresh installations.
    if config.ffmpeg_path.trim().is_empty() {
        return std::env::var_os(if name == "ffmpeg" {
            "LISEK_FFMPEG"
        } else {
            "LISEK_FFPROBE"
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| name.into());
    }
    let path = PathBuf::from(config.ffmpeg_path.trim());
    let executable = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    if path.is_dir() {
        if path.join(&executable).is_file() {
            path.join(executable)
        } else {
            path.join("bin").join(executable)
        }
    } else if name == "ffprobe" {
        path.with_file_name(executable)
    } else {
        path
    }
}
pub(crate) fn command(name: &str, config: &PlaybackSettings) -> Command {
    let mut command = Command::new(binary_path(name, config));
    command.kill_on_drop(true);
    // GPU numbers in the settings and in load balancing are `nvidia-smi`'s
    // (PCI order); CUDA's default order is "fastest first".
    if std::env::var_os("CUDA_DEVICE_ORDER").is_none() {
        command.env("CUDA_DEVICE_ORDER", "PCI_BUS_ID");
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

/// The parts of an FFmpeg build the planner chooses between. Filled by
/// [`tools`] from `ffmpeg -filters/-encoders/-hwaccels`.
#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct FfmpegTools {
    pub version: String,
    /// jellyfin-ffmpeg (adds `tonemap_cuda` with BT.2390 and Dolby Vision).
    pub jellyfin_build: bool,
    pub filters: BTreeSet<String>,
    pub encoders: BTreeSet<String>,
    pub hwaccels: BTreeSet<String>,
    /// Quick Sync decoders (`h264_qsv` ...), from `ffmpeg -decoders`.
    pub decoders: BTreeSet<String>,
}

/// Filters, encoders and hardware methods the planner may use. Anything
/// else in the build is irrelevant and left out of [`FfmpegTools`].
const INTERESTING_FILTERS: &[&str] = &[
    "tonemap_cuda",
    "tonemap_opencl",
    "libplacebo",
    "zscale",
    "tonemap",
    "scale_cuda",
    "yadif_cuda",
    "bwdif_cuda",
    "overlay_cuda",
    "subtitles",
    "hwupload_cuda",
    "scale_qsv",
    "vpp_qsv",
    "scale_vaapi",
];
const INTERESTING_ENCODERS: &[&str] = &[
    "libx264", "libx265", "h264_nvenc", "hevc_nvenc", "h264_qsv", "hevc_qsv", "h264_vaapi",
    "hevc_vaapi", "aac", "libfdk_aac", "ac3", "eac3", "webvtt",
];

impl FfmpegTools {
    pub fn has_filter(&self, name: &str) -> bool {
        self.filters.contains(name)
    }
    pub fn has_encoder(&self, name: &str) -> bool {
        self.encoders.contains(name)
    }
    pub fn has_decoder(&self, name: &str) -> bool {
        self.decoders.contains(name)
    }

    /// Adds the Quick Sync decoders listed by `ffmpeg -decoders`.
    pub fn with_decoders(mut self, decoders: &str) -> Self {
        self.decoders = decoders
            .lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .filter(|name| name.ends_with("_qsv"))
            .map(str::to_owned)
            .collect();
        self
    }

    /// Parses `ffmpeg -version`, `-filters`, `-encoders` and `-hwaccels`
    /// output. Pure, for tests.
    pub fn parse(version: &str, filters: &str, encoders: &str, hwaccels: &str) -> Self {
        let names = |text: &str, wanted: &[&str]| -> BTreeSet<String> {
            text.lines()
                // " T.C tonemap_cuda  V->V  ..." / " V....D h264_nvenc  NVIDIA ..."
                .filter_map(|line| line.split_whitespace().nth(1))
                .filter(|name| wanted.contains(name))
                .map(str::to_owned)
                .collect()
        };
        let first = version.lines().next().unwrap_or_default().to_owned();
        Self {
            jellyfin_build: version.to_ascii_lowercase().contains("jellyfin"),
            version: first,
            filters: names(filters, INTERESTING_FILTERS),
            encoders: names(encoders, INTERESTING_ENCODERS),
            hwaccels: hwaccels
                .lines()
                .skip_while(|l| !l.starts_with("Hardware acceleration methods"))
                .skip(1)
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect(),
            decoders: BTreeSet::new(),
        }
    }
}

struct CachedTools {
    binary: PathBuf,
    at: Instant,
    tools: FfmpegTools,
}
static TOOLS: LazyLock<Mutex<Option<CachedTools>>> = LazyLock::new(|| Mutex::new(None));
const TOOLS_TTL: Duration = Duration::from_secs(600);

async fn ffmpeg_output(config: &PlaybackSettings, args: &[&str]) -> Option<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        command("ffmpeg", config).args(["-hide_banner"]).args(args).output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        crate::tool_log::stderr(crate::tool_log::Tool::Ffmpeg, &args.join(" "), &output.stderr);
    }
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// What the configured FFmpeg build supports, cached for ten minutes per
/// binary. An unreachable binary yields an empty set (everything falls
/// back to software, and starting FFmpeg reports the real error).
pub async fn tools(config: &PlaybackSettings) -> FfmpegTools {
    let binary = binary_path("ffmpeg", config);
    if let Some(cached) = TOOLS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && cached.binary == binary
        && cached.at.elapsed() < TOOLS_TTL
    {
        return cached.tools.clone();
    }
    let (version, filters, encoders, hwaccels, decoders) = tokio::join!(
        ffmpeg_output(config, &["-version"]),
        ffmpeg_output(config, &["-filters"]),
        ffmpeg_output(config, &["-encoders"]),
        ffmpeg_output(config, &["-hwaccels"]),
        ffmpeg_output(config, &["-decoders"]),
    );
    let tools = FfmpegTools::parse(
        version.as_deref().unwrap_or_default(),
        filters.as_deref().unwrap_or_default(),
        encoders.as_deref().unwrap_or_default(),
        hwaccels.as_deref().unwrap_or_default(),
    )
    .with_decoders(decoders.as_deref().unwrap_or_default());
    if version.is_some() {
        *TOOLS.lock().unwrap_or_else(|e| e.into_inner()) = Some(CachedTools {
            binary,
            at: Instant::now(),
            tools: tools.clone(),
        });
    }
    tools
}

/// Forgets the cached [`tools`] (after the administrator changed the
/// FFmpeg path or installed a new build).
pub fn forget_tools() {
    *TOOLS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Capabilities {
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub version: String,
    pub jellyfin_build: bool,
    /// A test frame encoded with `h264_nvenc` on the configured GPU.
    pub nvenc_available: bool,
    /// A Vulkan device opens, so `libplacebo` can tone map on the GPU.
    pub vulkan_available: bool,
    pub software_tonemapping: bool,
    /// Tone mapping backends usable with this build and hardware, fastest
    /// first. `auto` takes the first one.
    pub tonemapping_backends: Vec<String>,
    pub filters: Vec<String>,
    pub encoders: Vec<String>,
    pub hwaccels: Vec<String>,
}

async fn succeeds(config: &PlaybackSettings, args: &[&str]) -> bool {
    tokio::time::timeout(
        Duration::from_secs(15),
        command("ffmpeg", config).args(args).output(),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .is_some_and(|o| o.status.success())
}

pub async fn capabilities(config: &PlaybackSettings) -> Result<Capabilities, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        command("ffmpeg", config).arg("-version").output(),
    )
    .await
    .map_err(|_| "ffmpeg_timed_out")?
    .map_err(|_| "ffmpeg_unavailable")?;
    if !output.status.success() {
        return Err("ffmpeg_unavailable".into());
    }
    let probe = tokio::time::timeout(
        Duration::from_secs(10),
        command("ffprobe", config).arg("-version").output(),
    )
    .await
    .map_err(|_| "ffprobe_timed_out")?
    .map_err(|_| "ffprobe_unavailable")?;
    if !probe.status.success() {
        return Err("ffprobe_unavailable".into());
    }
    forget_tools();
    let tools = tools(config).await;
    let device = config.hardware_device_index.to_string();
    let cuda = format!("cuda=cu:{device}");
    let vulkan = format!("vulkan=vk:{device}");
    let nvenc_args = [
        "-v", "error", "-init_hw_device", &cuda, "-f", "lavfi", "-i", "color=size=320x240:rate=24",
        "-frames:v", "1", "-c:v", "h264_nvenc", "-gpu", &device, "-f", "null", "-",
    ];
    let vulkan_args = [
        "-v", "error", "-init_hw_device", &vulkan, "-f", "lavfi", "-i", "nullsrc=size=16x16",
        "-frames:v", "1", "-f", "null", "-",
    ];
    let (nvenc, vulkan_ok) =
        tokio::join!(succeeds(config, &nvenc_args), succeeds(config, &vulkan_args));
    let backends = crate::tonemap::available_backends(&tools, nvenc, vulkan_ok)
        .into_iter()
        .map(|b| b.as_str().to_owned())
        .collect();
    Ok(Capabilities {
        ffmpeg_path: binary_path("ffmpeg", config).display().to_string(),
        ffprobe_path: binary_path("ffprobe", config).display().to_string(),
        version: tools.version.clone(),
        jellyfin_build: tools.jellyfin_build,
        nvenc_available: nvenc,
        vulkan_available: vulkan_ok,
        software_tonemapping: tools.has_filter("zscale") && tools.has_filter("tonemap"),
        tonemapping_backends: backends,
        filters: tools.filters.iter().cloned().collect(),
        encoders: tools.encoders.iter().cloned().collect(),
        hwaccels: tools.hwaccels.iter().cloned().collect(),
    })
}
/// Probes `path` and derives the transcoder's view of it. Kept for callers
/// that have no stored probe; the scanner and playback endpoints use
/// [`probe_media`] + [`source_info`] so the result can be cached.
pub async fn probe(path: &Path, config: &PlaybackSettings) -> Result<SourceInfo, String> {
    source_info(&probe_media(path, config).await?)
}

/// Runs ffprobe once and returns every stream. Error codes:
/// `ffprobe_unavailable` (binary missing, a configuration problem),
/// `probe_timed_out`, `probe_failed` (the file itself is unreadable).
pub async fn probe_media(path: &Path, config: &PlaybackSettings) -> Result<MediaProbe, String> {
    let mut cmd = command("ffprobe", config);
    cmd.args(["-v", "error", "-show_streams", "-show_format", "-show_chapters", "-of", "json"])
        .arg(path);
    let output = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| "probe_timed_out")?
        .map_err(|_| "ffprobe_unavailable")?;
    crate::tool_log::stderr(crate::tool_log::Tool::Ffprobe, &path.display().to_string(), &output.stderr);
    if !output.status.success() {
        return Err("probe_failed".into());
    }
    parse_probe(&output.stdout)
}

/// Pure JSON-to-domain conversion, split out for unit tests.
pub fn parse_probe(json: &[u8]) -> Result<MediaProbe, String> {
    let probe: Probe = serde_json::from_slice(json).map_err(|_| "probe_failed")?;
    let format = probe.format.unwrap_or_default();
    let streams = probe
        .streams
        .into_iter()
        .enumerate()
        .map(|(position, s)| {
            let flag = |name: &str| s.disposition.get(name).copied().unwrap_or(0) == 1;
            let tag = |name: &str| {
                s.tags
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.clone())
                    .filter(|v| !v.is_empty())
            };
            let dovi_profile = s
                .side_data_list
                .iter()
                .filter(|d| {
                    d.side_data_type
                        .as_deref()
                        .is_some_and(|t| t.contains("DOVI configuration"))
                })
                .find_map(|d| d.dv_profile);
            MediaStream {
                index: s.index.unwrap_or(position as i32),
                kind: StreamKind::parse(s.codec_type.as_deref().unwrap_or("data")),
                is_default: flag("default"),
                is_forced: flag("forced"),
                language: tag("language"),
                title: tag("title"),
                codec: s.codec_name,
                profile: s.profile,
                width: s.width,
                height: s.height,
                bit_depth: s.bits_per_raw_sample.and_then(|v| v.parse().ok()),
                pixel_format: s.pix_fmt,
                color_transfer: s.color_transfer,
                color_primaries: s.color_primaries,
                color_space: s.color_space,
                field_order: s.field_order,
                frame_rate: s
                    .avg_frame_rate
                    .as_deref()
                    .and_then(parse_rational)
                    .or_else(|| s.r_frame_rate.as_deref().and_then(parse_rational)),
                channels: s.channels,
                channel_layout: s.channel_layout,
                sample_rate: s.sample_rate.and_then(|v| v.parse().ok()),
                bitrate_kbps: s.bit_rate.as_deref().and_then(kbps),
                dovi_profile,
            }
        })
        .collect();
    let duration = format
        .duration
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0);
    let start_time = format
        .start_time
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(0.0);
    Ok(MediaProbe {
        container: format.format_name,
        duration_seconds: duration,
        bitrate_kbps: format.bit_rate.as_deref().and_then(kbps),
        start_time_seconds: Some(start_time),
        chapters: chapters_from(probe.chapters, start_time, duration),
        streams,
    })
}

/// Most chapters a file may contribute (a broken file can list thousands).
pub const MAX_CHAPTERS: usize = 200;

/// Chapters in playback time, sorted, without duplicates or ones starting
/// past the end. Titles that only number the chapter ("Chapter 03",
/// "00:12:30.000") are dropped, so the UI can name them consistently.
fn chapters_from(raw: Vec<Chapter>, start_time: f64, duration: Option<f64>) -> Vec<MediaChapter> {
    let time = |v: Option<&str>| v.and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite());
    let mut chapters: Vec<MediaChapter> = raw
        .into_iter()
        .filter_map(|c| {
            let start = (time(c.start_time.as_deref())? - start_time).max(0.0);
            if duration.is_some_and(|d| start >= d - 1.0) {
                return None;
            }
            let end = time(c.end_time.as_deref()).map(|e| (e - start_time).max(start));
            let title = c
                .tags
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("title"))
                .map(|(_, v)| v.trim().to_owned())
                .filter(|v| !v.is_empty() && !is_placeholder_title(v));
            Some(MediaChapter { start_seconds: start, end_seconds: end, title })
        })
        .collect();
    chapters.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    chapters.dedup_by(|a, b| (a.start_seconds - b.start_seconds).abs() < 0.5);
    chapters.truncate(MAX_CHAPTERS);
    chapters
}

fn is_placeholder_title(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    let rest = ["chapter", "rozdział", "rozdzial", "kapitola", "chapitre", "kapitel"]
        .iter()
        .find_map(|word| lower.strip_prefix(word))
        .unwrap_or(&lower);
    rest.trim().chars().all(|c| c.is_ascii_digit() || matches!(c, ':' | '.' | ' '))
}

/// The transcoder's view of a probed file. Fails the same way the old
/// inline probe did, so callers keep their error codes.
pub fn source_info(probe: &MediaProbe) -> Result<SourceInfo, String> {
    let video = probe.primary_video().ok_or("video_stream_missing")?;
    let duration = probe.duration_seconds.ok_or("duration_unavailable")?;
    let dimension = |v: Option<i32>| {
        v.and_then(|v| u32::try_from(v).ok())
            .ok_or("resolution_unavailable")
    };
    Ok(SourceInfo {
        width: dimension(video.width)?,
        height: dimension(video.height)?,
        bitrate_kbps: probe
            .bitrate_kbps
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0),
        duration,
        codec: video.codec.clone().unwrap_or_default(),
        pixel_format: video.pixel_format.clone().unwrap_or_default(),
        hdr: video.hdr_format().is_some(),
        interlaced: video.is_interlaced(),
        audio_channels: probe
            .primary_audio()
            .and_then(|a| a.channels)
            .and_then(|c| u32::try_from(c).ok())
            .unwrap_or(0),
        video_index: Some(video.index),
    })
}

fn parse_rational(value: &str) -> Option<f64> {
    let (num, den) = value.split_once('/').unwrap_or((value, "1"));
    let (num, den) = (num.parse::<f64>().ok()?, den.parse::<f64>().ok()?);
    (den != 0.0 && num > 0.0).then(|| num / den)
}

fn kbps(bits_per_second: &str) -> Option<i32> {
    let bps = bits_per_second.parse::<u64>().ok()?;
    Some((bps / 1000).min(i32::MAX as u64) as i32)
}

#[cfg(test)]
mod tools_tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_listings() {
        let filters = " ... aresample         A->A       Resample audio data.\n \
            ..C libplacebo        N->V       Apply various GPU filters\n \
            ... tonemap_cuda      V->V       GPU accelerated HDR to SDR tonemapping\n \
            .S. tonemap           V->V       Conversion to/from different dynamic ranges.\n";
        let encoders = " V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)\n \
            A....D aac                  AAC (Advanced Audio Coding)\n";
        let hwaccels = "Hardware acceleration methods:\ncuda\nvulkan\n\n";
        let tools = FfmpegTools::parse(
            "ffmpeg version 7.1.2-Jellyfin Copyright (c) 2000-2025\nbuilt with gcc",
            filters,
            encoders,
            hwaccels,
        );
        assert!(tools.jellyfin_build);
        assert_eq!(tools.version, "ffmpeg version 7.1.2-Jellyfin Copyright (c) 2000-2025");
        assert!(tools.has_filter("tonemap_cuda") && tools.has_filter("libplacebo"));
        assert!(!tools.has_filter("aresample"), "only filters the planner uses are kept");
        assert!(tools.has_encoder("h264_nvenc") && tools.has_encoder("aac"));
        assert_eq!(tools.hwaccels.iter().collect::<Vec<_>>(), ["cuda", "vulkan"]);
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "streams": [
        {"index":0,"codec_type":"video","codec_name":"mjpeg","width":600,"height":900,
         "disposition":{"attached_pic":1}},
        {"index":1,"codec_type":"video","codec_name":"hevc","profile":"Main 10","width":3840,"height":2160,
         "pix_fmt":"yuv420p10le","color_transfer":"smpte2084","color_primaries":"bt2020","color_space":"bt2020nc",
         "field_order":"progressive","avg_frame_rate":"24000/1001","r_frame_rate":"24000/1001",
         "bits_per_raw_sample":"10","disposition":{"default":1},
         "side_data_list":[{"side_data_type":"DOVI configuration record","dv_version_major":1,"dv_profile":8}]},
        {"index":2,"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000",
         "bit_rate":"192000","tags":{"language":"eng"},"disposition":{"default":0}},
        {"index":3,"codec_type":"audio","codec_name":"eac3","channels":6,"channel_layout":"5.1(side)",
         "sample_rate":"48000","tags":{"language":"pol","title":"Lektor"},"disposition":{"default":1}},
        {"index":4,"codec_type":"subtitle","codec_name":"subrip","tags":{"LANGUAGE":"pol"},
         "disposition":{"default":0,"forced":1}}
      ],
      "format": {"format_name":"matroska,webm","duration":"5421.120000","bit_rate":"24500123","start_time":"-0.021000"}
    }"#;

    #[test]
    fn parses_every_stream_with_tags_and_dispositions() {
        let probe = parse_probe(SAMPLE.as_bytes()).unwrap();
        assert_eq!(probe.container.as_deref(), Some("matroska,webm"));
        assert_eq!(probe.duration_seconds, Some(5421.12));
        assert_eq!(probe.bitrate_kbps, Some(24500));
        assert_eq!(probe.start_time_seconds, Some(-0.021));
        assert_eq!(probe.streams.len(), 5);
        let video = &probe.streams[1];
        assert_eq!(video.bit_depth, Some(10));
        assert_eq!(video.dovi_profile, Some(8));
        assert!((video.frame_rate.unwrap() - 23.976).abs() < 0.001);
        let lektor = &probe.streams[3];
        assert_eq!(lektor.language.as_deref(), Some("pol"));
        assert_eq!(lektor.title.as_deref(), Some("Lektor"));
        assert!(lektor.is_default);
        let subtitle = &probe.streams[4];
        assert_eq!(subtitle.kind, StreamKind::Subtitle);
        assert_eq!(subtitle.language.as_deref(), Some("pol"), "tag keys are case-insensitive");
        assert!(subtitle.is_forced);
    }

    #[test]
    fn source_info_skips_cover_art_and_uses_default_audio() {
        let info = source_info(&parse_probe(SAMPLE.as_bytes()).unwrap()).unwrap();
        assert_eq!((info.width, info.height), (3840, 2160));
        assert_eq!(info.video_index, Some(1));
        assert!(info.hdr);
        assert!(!info.interlaced);
        assert_eq!(info.audio_channels, 6);
        assert_eq!(info.bitrate_kbps, 24500);
    }

    #[test]
    fn source_info_reports_the_same_error_codes_as_before() {
        let no_video = r#"{"streams":[{"index":0,"codec_type":"audio"}],"format":{"duration":"10"}}"#;
        assert_eq!(source_info(&parse_probe(no_video.as_bytes()).unwrap()).unwrap_err(), "video_stream_missing");
        let no_duration = r#"{"streams":[{"index":0,"codec_type":"video","width":1,"height":1}],"format":{}}"#;
        assert_eq!(source_info(&parse_probe(no_duration.as_bytes()).unwrap()).unwrap_err(), "duration_unavailable");
        assert_eq!(parse_probe(b"not json").unwrap_err(), "probe_failed");
    }

    #[test]
    fn chapters_are_read_in_playback_time() {
        let json = r#"{
            "format": {"duration": "600.0", "start_time": "1.000"},
            "streams": [],
            "chapters": [
                {"start_time": "301.000", "end_time": "600.000", "tags": {"title": "Pościg"}},
                {"start_time": "1.000", "end_time": "301.000", "tags": {"title": "Chapter 01"}},
                {"start_time": "301.200", "end_time": "302.000", "tags": {"title": "dup"}},
                {"start_time": "600.500", "tags": {"title": "after the end"}},
                {"start_time": "x"}
            ]
        }"#;
        let probe = parse_probe(json.as_bytes()).unwrap();
        assert_eq!(
            probe.chapters,
            vec![
                MediaChapter { start_seconds: 0.0, end_seconds: Some(300.0), title: None },
                MediaChapter { start_seconds: 300.0, end_seconds: Some(599.0), title: Some("Pościg".into()) },
            ]
        );
        assert!(is_placeholder_title("Rozdział 12"));
        assert!(is_placeholder_title("00:12:30.000"));
        assert!(!is_placeholder_title("Chapter of Doom"));
        assert!(parse_probe(br#"{"streams": []}"#).unwrap().chapters.is_empty());
    }
}
