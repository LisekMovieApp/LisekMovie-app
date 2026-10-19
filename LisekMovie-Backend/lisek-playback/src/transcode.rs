//! Bounded FFmpeg jobs. Configuration is snapshotted when each job starts.
use lisek_core::media_probe::{MediaProbe, MediaStream, StreamKind};
use lisek_core::playback_settings::PlaybackSettings;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::LazyLock,
    time::{Duration, Instant},
};
use tokio::{
    process::{Child, Command},
    sync::Mutex,
};

struct Job {
    child: Child,
    directory: PathBuf,
    touched: Instant,
}
static JOBS: LazyLock<Mutex<HashMap<String, Job>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

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
}
#[derive(Deserialize)]
struct Format {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
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
fn command(name: &str, config: &PlaybackSettings) -> Command {
    let mut command = Command::new(binary_path(name, config));
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}
#[derive(Serialize)]
pub struct Capabilities {
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
    pub version: String,
    pub nvenc_available: bool,
    pub software_tonemapping: bool,
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
    let nvenc = tokio::time::timeout(
        Duration::from_secs(15),
        command("ffmpeg", config)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=size=320x240:rate=24",
                "-frames:v",
                "1",
                "-c:v",
                "h264_nvenc",
                "-f",
                "null",
                "-",
            ])
            .output(),
    )
    .await;
    let filters = tokio::time::timeout(
        Duration::from_secs(10),
        command("ffmpeg", config).args(["-filters"]).output(),
    )
    .await;
    let filters = filters
        .ok()
        .and_then(Result::ok)
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    Ok(Capabilities {
        ffmpeg_path: binary_path("ffmpeg", config).display().to_string(),
        ffprobe_path: binary_path("ffprobe", config).display().to_string(),
        version: String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned(),
        nvenc_available: nvenc
            .ok()
            .and_then(Result::ok)
            .is_some_and(|o| o.status.success()),
        software_tonemapping: filters.contains(" zscale ") && filters.contains(" tonemap "),
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
    cmd.args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path);
    let output = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| "probe_timed_out")?
        .map_err(|_| "ffprobe_unavailable")?;
    if !output.status.success() {
        return Err("probe_failed".into());
    }
    parse_probe(&output.stdout)
}

/// Pure JSON-to-domain conversion, split out for unit tests.
pub fn parse_probe(json: &[u8]) -> Result<MediaProbe, String> {
    let probe: Probe = serde_json::from_slice(json).map_err(|_| "probe_failed")?;
    let format = probe.format.unwrap_or(Format {
        format_name: None,
        duration: None,
        bit_rate: None,
    });
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
            }
        })
        .collect();
    Ok(MediaProbe {
        container: format.format_name,
        duration_seconds: format
            .duration
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0),
        bitrate_kbps: format.bit_rate.as_deref().and_then(kbps),
        streams,
    })
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
        hdr: matches!(
            video.color_transfer.as_deref(),
            Some("smpte2084" | "arib-std-b67")
        ),
        interlaced: matches!(
            video.field_order.as_deref(),
            Some("tt" | "bb" | "tb" | "bt")
        ),
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

fn encode_command(
    path: &Path,
    directory: &Path,
    offset: f64,
    config: &PlaybackSettings,
    source: &SourceInfo,
    remote_limit: Option<u32>,
) -> Result<Command, String> {
    if !config.validate() {
        return Err("invalid_playback_settings".into());
    }
    // Reserve headroom for the container and constrain audio as well as video.
    let total = remote_limit.map(|n| n.saturating_mul(90) / 100);
    let audio = config.audio_bitrate_kbps.min(
        total
            .map(|n| (n / 4).clamp(32, 128))
            .unwrap_or(config.audio_bitrate_kbps),
    );
    let video = total
        .map(|n| n.saturating_sub(audio))
        .unwrap_or(config.video_bitrate_kbps)
        .min(config.video_bitrate_kbps);
    if video < 100 {
        return Err("streaming_limit_too_low".into());
    }
    let mut cmd = command("ffmpeg", config);
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "warning",
        "-nostdin",
        "-y",
        "-ss",
        &offset.to_string(),
        "-re",
    ]);
    let ten_bit = source.pixel_format.contains("10");
    let decode = config.hardware_acceleration == "nvenc"
        && config.hardware_decoding_codecs.contains(&source.codec)
        && !source.pixel_format.contains("12")
        && !source.pixel_format.contains("422")
        && !source.pixel_format.contains("444")
        && !(ten_bit && source.codec == "hevc" && !config.decode_hevc_10bit)
        && !(ten_bit && source.codec == "vp9" && !config.decode_vp9_10bit);
    if decode {
        cmd.args(["-hwaccel", "cuda"]);
        if !config.enhanced_nvdec {
            let codec = match source.codec.as_str() {
                "mpeg2video" => "mpeg2",
                s => s,
            };
            cmd.args(["-c:v", &format!("{codec}_cuvid")]);
        }
    }
    cmd.args(["-threads", &config.encoding_threads.to_string(), "-i"])
        .arg(path)
        .args([
            "-map",
            &source
                .video_index
                .map_or_else(|| "0:v:0".to_owned(), |index| format!("0:{index}")),
            "-map",
            "0:a:0?",
            "-sn",
        ]);
    if config.hardware_acceleration == "nvenc" && config.hardware_encoding {
        let preset = match config.encoder_preset.as_str() {
            "ultrafast" | "superfast" => "p1",
            "veryfast" | "faster" => "p2",
            "fast" => "p3",
            "medium" => "p4",
            "slow" => "p5",
            "slower" => "p6",
            _ => "p7",
        };
        cmd.args([
            "-c:v",
            "h264_nvenc",
            "-preset",
            preset,
            "-rc",
            "vbr",
            "-b:v",
            &format!("{video}k"),
        ]);
    } else {
        cmd.args([
            "-c:v",
            "libx264",
            "-preset",
            &config.encoder_preset,
            "-crf",
            &config.h264_crf.to_string(),
        ]);
    }
    let mut filters = Vec::new();
    if source.interlaced && config.deinterlace_method != "none" {
        filters.push(format!(
            "{}=mode={}:parity=auto:deint=interlaced",
            config.deinterlace_method,
            if config.deinterlace_double_rate {
                "send_field"
            } else {
                "send_frame"
            }
        ));
    }
    if source.hdr && config.tonemapping {
        let mut tonemap = format!(
            "tonemap=tonemap={}:desat={}:peak={}",
            config.tonemapping_algorithm, config.tonemapping_desat, config.tonemapping_peak
        );
        if let Some(param) = config.tonemapping_param {
            tonemap.push_str(&format!(":param={param}"));
        }
        filters.extend([
            "zscale=transfer=linear:npl=100".into(),
            "format=gbrpf32le".into(),
            "zscale=primaries=bt709".into(),
            tonemap,
            "zscale=transfer=bt709:matrix=bt709:range=limited".into(),
        ]);
        cmd.args([
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ]);
    }
    // Fit a 16:9 bounding box without enlarging small sources; preserve portrait aspect ratios.
    let height = config.max_output_height.min(source.height).max(2);
    let width = (config.max_output_height * 16 / 9).min(source.width).max(2);
    filters.push(format!(
        "scale=w={width}:h={height}:force_original_aspect_ratio=decrease:force_divisible_by=2"
    ));
    cmd.args([
        "-vf",
        &filters.join(","),
        "-pix_fmt",
        "yuv420p",
        "-threads",
        &config.encoding_threads.to_string(),
        "-maxrate",
        &format!("{video}k"),
        "-bufsize",
        &format!("{}k", video * 2),
        "-c:a",
        "aac",
        "-ac",
        "2",
    ]);
    if config.audio_vbr && remote_limit.is_none() {
        cmd.args(["-q:a", "2"]);
    } else {
        cmd.args(["-b:a", &format!("{audio}k")]);
    }
    if source.audio_channels > 2 {
        cmd.args(["-af", &format!("volume={}", config.downmix_audio_boost)]);
    }
    cmd.args([
        "-max_muxing_queue_size",
        &config.max_muxing_queue_size.to_string(),
        "-force_key_frames",
        "expr:gte(t,n_forced*4)",
        "-f",
        "hls",
        "-hls_time",
        "4",
        "-hls_list_size",
        "0",
        "-hls_playlist_type",
        "event",
        "-hls_flags",
        "temp_file",
        "-hls_segment_filename",
    ])
    .arg(directory.join("segment-%06d.ts"))
    .arg(directory.join("index.m3u8"));
    Ok(cmd)
}
pub async fn start(
    id: &str,
    path: &Path,
    directory: &Path,
    offset: f64,
    config: &PlaybackSettings,
    source: &SourceInfo,
    remote_limit: Option<u32>,
) -> Result<(), String> {
    let mut jobs = JOBS.lock().await;
    let active = jobs
        .values_mut()
        .map(|job| usize::from(job.child.try_wait().ok().flatten().is_none()))
        .sum::<usize>();
    if active >= 2 {
        return Err("transcode_capacity_reached".into());
    }
    let mut cmd = encode_command(path, directory, offset, config, source, remote_limit)?;
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(|_| "transcode_directory_unavailable")?;
    let log = std::fs::File::create(directory.join("ffmpeg.log"))
        .map_err(|_| "transcode_log_unavailable")?;
    cmd.stdout(std::process::Stdio::null()).stderr(log);
    let child = cmd.spawn().map_err(|_| "ffmpeg_unavailable")?;
    jobs.insert(
        id.to_owned(),
        Job {
            child,
            directory: directory.to_owned(),
            touched: Instant::now(),
        },
    );
    Ok(())
}
pub async fn file_path(id: &str, file: &str) -> Option<PathBuf> {
    if !valid_file(file) {
        return None;
    }
    JOBS.lock().await.get(id).map(|j| j.directory.join(file))
}
pub async fn failed(id: &str) -> bool {
    JOBS.lock().await.get_mut(id).is_none_or(|j| {
        j.child
            .try_wait()
            .ok()
            .flatten()
            .is_some_and(|s| !s.success())
    })
}
pub async fn touch(id: &str) -> bool {
    if let Some(job) = JOBS.lock().await.get_mut(id) {
        job.touched = Instant::now();
        true
    } else {
        false
    }
}
pub async fn exists(id: &str) -> bool {
    JOBS.lock().await.contains_key(id)
}
pub async fn stop(id: &str) {
    if let Some(mut job) = JOBS.lock().await.remove(id) {
        let _ = job.child.kill().await;
        let _ = job.child.wait().await;
        let _ = tokio::fs::remove_dir_all(&job.directory).await;
    }
}
pub async fn cleanup() {
    let ids: Vec<String> = JOBS
        .lock()
        .await
        .iter()
        .filter(|(_, job)| job.touched.elapsed() > Duration::from_secs(90))
        .map(|(id, _)| id.clone())
        .collect();
    for id in ids {
        stop(&id).await;
    }
}
pub fn valid_file(name: &str) -> bool {
    name == "index.m3u8"
        || name
            .strip_prefix("segment-")
            .and_then(|s| s.strip_suffix(".ts"))
            .is_some_and(|n| n.len() == 6 && n.bytes().all(|b| b.is_ascii_digit()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_allows_generated_media_filenames() {
        for name in ["index.m3u8", "segment-000012.ts"] {
            assert!(valid_file(name));
        }
        for name in [
            "../index.m3u8",
            "segment-../../secret.ts",
            "anything.ts",
            "segment-1.ts",
            "/index.m3u8",
            "ffmpeg.log",
        ] {
            assert!(!valid_file(name));
        }
    }
    #[test]
    fn remote_budget_overrides_vbr_and_config_reaches_encoder() {
        let config = PlaybackSettings {
            audio_vbr: true,
            encoding_threads: 3,
            encoder_preset: "slow".into(),
            h264_crf: 20,
            ..Default::default()
        };
        let source = SourceInfo {
            width: 1920,
            height: 1080,
            bitrate_kbps: 8000,
            duration: 600.0,
            codec: "h264".into(),
            pixel_format: "yuv420p".into(),
            hdr: false,
            interlaced: false,
            audio_channels: 6,
            video_index: None,
        };
        let cmd = encode_command(
            Path::new("input.mp4"),
            Path::new("output"),
            5.0,
            &config,
            &source,
            Some(1000),
        )
        .unwrap();
        let args: Vec<_> = cmd
            .as_std()
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();
        let value = |key: &str| args[args.iter().position(|s| s == key).unwrap() + 1].clone();
        assert_eq!(value("-maxrate"), "772k");
        assert_eq!(value("-b:a"), "128k");
        assert!(!args.contains(&"-q:a".to_owned()));
        assert_eq!(value("-crf"), "20");
        assert_eq!(value("-threads"), "3");
        assert_eq!(value("-preset"), "slow");
        assert_eq!(value("-af"), "volume=2");
        assert!(
            encode_command(
                Path::new("in"),
                Path::new("out"),
                0.0,
                &config,
                &source,
                Some(100)
            )
            .is_err()
        );
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
         "bits_per_raw_sample":"10","disposition":{"default":1}},
        {"index":2,"codec_type":"audio","codec_name":"aac","channels":2,"sample_rate":"48000",
         "bit_rate":"192000","tags":{"language":"eng"},"disposition":{"default":0}},
        {"index":3,"codec_type":"audio","codec_name":"eac3","channels":6,"channel_layout":"5.1(side)",
         "sample_rate":"48000","tags":{"language":"pol","title":"Lektor"},"disposition":{"default":1}},
        {"index":4,"codec_type":"subtitle","codec_name":"subrip","tags":{"LANGUAGE":"pol"},
         "disposition":{"default":0,"forced":1}}
      ],
      "format": {"format_name":"matroska,webm","duration":"5421.120000","bit_rate":"24500123"}
    }"#;

    #[test]
    fn parses_every_stream_with_tags_and_dispositions() {
        let probe = parse_probe(SAMPLE.as_bytes()).unwrap();
        assert_eq!(probe.container.as_deref(), Some("matroska,webm"));
        assert_eq!(probe.duration_seconds, Some(5421.12));
        assert_eq!(probe.bitrate_kbps, Some(24500));
        assert_eq!(probe.streams.len(), 5);
        let video = &probe.streams[1];
        assert_eq!(video.bit_depth, Some(10));
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
}
