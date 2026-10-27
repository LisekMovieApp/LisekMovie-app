//! Direct play, remux or transcode: the cheapest way to get a file to a
//! client, decided per stream.
//!
//! - **Direct play**: the client opens the file itself (range requests).
//! - **Remux**: the video is copied (`-c:v copy`) into fragmented-MP4 HLS;
//!   only the audio may be re-encoded. Costs a little I/O, almost no CPU.
//! - **Transcode**: the video is re-encoded (NVENC or x264).
//!
//! Every reason that pushed playback away from direct play is reported,
//! so the player can show why and the administrator can see what costs
//! CPU.

use lisek_core::media_probe::{HdrFormat, MediaProbe, MediaStream, StreamKind};
use lisek_core::playback_settings::PlaybackSettings;
use serde::Serialize;
use std::path::PathBuf;

use crate::downmix::{self, DownmixAlgorithm, DownmixPreference};
use crate::profile::{ClientCapabilities, HlsSupport};
use crate::subtitles::{self, ExternalSubtitle, SubtitleKind, TrackRef};
use crate::tonemap::{self, TonemapBackend};
use crate::transcode::FfmpegTools;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlayMethod {
    DirectPlay,
    Remux,
    Transcode,
}

impl PlayMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            PlayMethod::DirectPlay => "direct_play",
            PlayMethod::Remux => "remux",
            PlayMethod::Transcode => "transcode",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscodeReason {
    ForcedByClient,
    ContainerNotSupported,
    VideoCodecNotSupported,
    VideoBitDepthNotSupported,
    VideoProfileNotSupported,
    HdrNotSupported,
    InterlacedNotSupported,
    ResolutionAboveRequested,
    BitrateAboveLimit,
    AudioCodecNotSupported,
    AudioChannelsNotSupported,
    AudioTrackNotFirst,
    DownmixRequested,
    SubtitleBurnIn,
    SubtitleNeedsHls,
    NoKeyframeIndex,
    RemuxDisabled,
}

/// The client's choices for one playback.
#[derive(Debug, Clone, Default)]
pub struct PlaybackRequest {
    pub audio_stream_index: Option<i32>,
    pub subtitle: Option<TrackRef>,
    /// Burn the subtitle in even when it is text (keeps ASS styling).
    pub burn_subtitle: bool,
    pub force_transcode: bool,
    pub max_height: Option<u32>,
    pub max_bitrate_kbps: Option<u32>,
    pub downmix: DownmixPreference,
}

pub struct DecisionInput<'a> {
    pub probe: &'a MediaProbe,
    pub externals: &'a [ExternalSubtitle],
    pub client: &'a ClientCapabilities,
    pub settings: &'a PlaybackSettings,
    pub tools: &'a FfmpegTools,
    pub request: &'a PlaybackRequest,
    pub remote_limit_kbps: Option<u32>,
    /// A keyframe index exists, so copied video can be cut into segments.
    pub keyframes_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputCodec {
    H264,
    Hevc,
}

impl OutputCodec {
    pub fn name(self) -> &'static str {
        match self {
            OutputCodec::H264 => "h264",
            OutputCodec::Hevc => "hevc",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TonemapPlan {
    pub backend: TonemapBackend,
    /// Algorithm name in the backend's own spelling.
    pub algorithm: String,
    pub peak: f64,
    pub desat: f64,
    pub param: Option<f64>,
    /// `smpte2084` (PQ) or `arib-std-b67` (HLG).
    pub transfer: &'static str,
    /// Apply Dolby Vision metadata (libplacebo, jellyfin's tonemap_cuda).
    pub dolby_vision: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BurnInSource {
    /// Embedded stream: ffprobe index and position among the file's
    /// subtitle streams (what the `subtitles` filter calls `si`).
    Embedded { stream_index: i32, ordinal: usize },
    External { path: PathBuf, format: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct BurnIn {
    pub source: BurnInSource,
    pub kind: SubtitleKind,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoEncode {
    pub codec: OutputCodec,
    pub width: u32,
    pub height: u32,
    pub bitrate_kbps: u32,
    pub gpu_decode: bool,
    pub gpu_encode: bool,
    pub tonemap: Option<TonemapPlan>,
    /// Keep HDR: 10-bit HEVC with BT.2020 tags, for HDR clients.
    pub hdr_passthrough: bool,
    pub deinterlace: Option<(String, bool)>,
    pub burn_in: Option<BurnIn>,
    pub frame_rate: f64,
    pub source_width: u32,
    pub source_height: u32,
    pub source_bit_depth: u32,
    /// ffprobe's codec name of the source (`h264`, `hevc` ...).
    pub source_codec: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VideoAction {
    Copy,
    Encode(Box<VideoEncode>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodecOut {
    Aac,
    Eac3,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AudioAction {
    Copy,
    Encode {
        codec: AudioCodecOut,
        bitrate_kbps: u32,
        channels: u32,
        downmix: Option<DownmixAlgorithm>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioPlan {
    pub stream_index: i32,
    pub action: AudioAction,
    pub source_codec: String,
    pub source_channels: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleDelivery {
    /// A WebVTT file for a `<track>` element (direct play).
    External,
    /// A WebVTT rendition in the HLS master playlist.
    Hls,
    BurnIn,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubtitlePlan {
    pub track: TrackRef,
    pub delivery: SubtitleDelivery,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaybackPlan {
    pub method: PlayMethod,
    pub reasons: Vec<TranscodeReason>,
    pub video_index: i32,
    pub video: VideoAction,
    pub audio: Option<AudioPlan>,
    pub subtitle: Option<SubtitlePlan>,
    /// Picture size, codec and bitrate the client receives.
    pub output_width: u32,
    pub output_height: u32,
    pub output_video_codec: String,
    pub output_bitrate_kbps: u32,
    pub source_hdr: Option<HdrFormat>,
}

impl PlaybackPlan {
    pub fn encodes_video(&self) -> bool {
        matches!(self.video, VideoAction::Encode(_))
    }

    /// Decodes or encodes video on the GPU (Faza 13: such sessions are
    /// spread over the GPUs, see `gpu`).
    pub fn uses_gpu(&self) -> bool {
        matches!(&self.video, VideoAction::Encode(e) if e.gpu_encode || e.gpu_decode)
    }
}

/// `mp4`, `webm`, `mkv`, `ts`, `avi` ... from ffprobe's `format_name`.
pub fn container_key(probe: &MediaProbe) -> String {
    let format = probe.container.as_deref().unwrap_or_default();
    if format.contains("matroska") {
        let webm_video = probe
            .primary_video()
            .and_then(|v| v.codec.as_deref())
            .is_some_and(|c| matches!(c, "vp8" | "vp9" | "av1"));
        let webm_audio = probe
            .streams
            .iter()
            .filter(|s| s.kind == StreamKind::Audio)
            .all(|s| matches!(s.codec.as_deref(), Some("opus" | "vorbis")));
        if webm_video && webm_audio { "webm" } else { "mkv" }.into()
    } else if format.contains("mp4") || format.contains("mov") {
        "mp4".into()
    } else if format.contains("mpegts") {
        "ts".into()
    } else {
        format.split(',').next().unwrap_or_default().to_owned()
    }
}

/// The client capability name of a video stream (`hevc_10bit` ...), or
/// `None` for formats no client decodes (4:2:2, 4:4:4, 12-bit).
pub fn video_capability(video: &MediaStream) -> Option<String> {
    let codec = video.codec.as_deref()?;
    let format = video.pixel_format.as_deref().unwrap_or_default();
    if format.contains("422") || format.contains("444") {
        return None;
    }
    match video.effective_bit_depth() {
        8 => Some(codec.to_owned()),
        10 => Some(format!("{codec}_10bit")),
        _ => None,
    }
}

/// The client capability name of an audio codec.
pub fn audio_capability(codec: &str) -> &str {
    if codec.starts_with("pcm_") { "pcm" } else { codec }
}

/// Video codecs that go into fragmented MP4 without re-encoding.
fn fmp4_video(codec: &str) -> bool {
    matches!(codec, "h264" | "hevc" | "av1" | "vp9")
}

/// Audio codecs that go into fragmented MP4 without re-encoding.
fn fmp4_audio(codec: &str) -> bool {
    matches!(codec, "aac" | "mp3" | "ac3" | "eac3" | "opus" | "flac" | "alac")
}

/// `(width, height)` fitting `source` into a 16:9 box `max_height` tall,
/// never upscaling, both even.
pub fn fit_dimensions(source_width: u32, source_height: u32, max_height: u32) -> (u32, u32) {
    let even = |v: f64| ((v / 2.0).round() as u32 * 2).max(2);
    let box_h = max_height.min(source_height.max(2)) as f64;
    let box_w = (max_height as f64 * 16.0 / 9.0).min(source_width.max(2) as f64);
    let scale = (box_w / source_width.max(1) as f64)
        .min(box_h / source_height.max(1) as f64)
        .min(1.0);
    (
        even(source_width as f64 * scale),
        even(source_height as f64 * scale),
    )
}

fn select_backend(settings: &PlaybackSettings, tools: &FfmpegTools) -> Option<TonemapBackend> {
    if matches!(hardware_api(settings), Some("qsv" | "vaapi")) {
        // The GPU tone mappers here are CUDA, Vulkan and OpenCL devices
        // numbered like NVIDIA cards; on an Intel/AMD session the CPU
        // tone maps and the hardware encoder takes the result.
        return tonemap::select("software", tools, false);
    }
    tonemap::select(
        &settings.tonemapping_backend,
        tools,
        settings.hardware_acceleration == "nvenc",
    )
}

/// The hardware API a session's settings use: `nvenc`, `qsv` or `vaapi`
/// (a pool is resolved to one of them per session before deciding).
pub fn hardware_api(settings: &PlaybackSettings) -> Option<&'static str> {
    match settings.hardware_acceleration.as_str() {
        "nvenc" => Some("nvenc"),
        "qsv" => Some("qsv"),
        "vaapi" => Some("vaapi"),
        _ => None,
    }
}

/// `h264_nvenc`, `hevc_qsv` ...
pub fn hardware_encoder(api: &str, codec: OutputCodec) -> String {
    let codec = match codec {
        OutputCodec::H264 => "h264",
        OutputCodec::Hevc => "hevc",
    };
    format!("{codec}_{api}")
}

/// Whether the hardware decoder of `api` takes this source: codec enabled
/// in the settings, supported layout, and the decoder in this FFmpeg build.
fn hardware_decodes(api: &str, settings: &PlaybackSettings, tools: &FfmpegTools, codec: &str, depth: u32, format: &str) -> bool {
    let common = settings.hardware_decoding_codecs.iter().any(|c| c == codec)
        && depth <= 10
        && !format.contains("422")
        && !format.contains("444")
        && !(depth == 10 && codec == "hevc" && !settings.decode_hevc_10bit)
        && !(depth == 10 && codec == "vp9" && !settings.decode_vp9_10bit);
    common
        && match api {
            "nvenc" => tools.hwaccels.contains("cuda"),
            // Quick Sync decodes with its own decoders (`h264_qsv`), which
            // know fewer codecs than the hwaccel list suggests.
            "qsv" => tools.hwaccels.contains("qsv") && tools.has_decoder(&format!("{}_qsv", qsv_decoder_codec(codec))),
            "vaapi" => tools.hwaccels.contains("vaapi") && !matches!(codec, "mpeg4" | "vp8"),
            _ => false,
        }
}

/// FFmpeg's Quick Sync decoder name stem for an ffprobe codec name.
pub fn qsv_decoder_codec(codec: &str) -> &str {
    match codec {
        "mpeg2video" => "mpeg2",
        other => other,
    }
}

/// Decides how `input.probe` plays on `input.client`.
pub fn decide(input: &DecisionInput) -> Result<PlaybackPlan, &'static str> {
    let DecisionInput { probe, client, settings, request, .. } = *input;
    let video = probe.primary_video().ok_or("video_stream_missing")?;
    let audio = match request.audio_stream_index {
        Some(index) => Some(
            probe
                .stream(index)
                .filter(|s| s.kind == StreamKind::Audio)
                .ok_or("audio_stream_not_found")?,
        ),
        None => probe.primary_audio(),
    };

    // --- subtitle ---------------------------------------------------------
    let subtitle = match request.subtitle {
        None => None,
        Some(track) => Some((track, subtitle_kind(probe, input.externals, track)?)),
    };
    let burn_in = subtitle.as_ref().and_then(|(track, kind)| {
        (*kind == SubtitleKind::Image || request.burn_subtitle)
            .then(|| burn_in_source(probe, input.externals, *track, *kind))
            .flatten()
    });

    // --- can the video be copied? ----------------------------------------
    let mut reasons = vec![];
    let mut video_reasons = vec![];
    if request.force_transcode {
        video_reasons.push(TranscodeReason::ForcedByClient);
    }
    match video_capability(video) {
        Some(cap) if client.video(&cap) => {}
        Some(cap) if cap.ends_with("_10bit") && client.video(video.codec.as_deref().unwrap_or("")) => {
            video_reasons.push(TranscodeReason::VideoBitDepthNotSupported)
        }
        Some(_) => video_reasons.push(TranscodeReason::VideoCodecNotSupported),
        None => video_reasons.push(TranscodeReason::VideoProfileNotSupported),
    }
    let hdr = video.hdr_format();
    if let Some(format) = hdr {
        let broken_colours = matches!(format, HdrFormat::DolbyVision { profile: 5 });
        if !client.shows_hdr(format) && (broken_colours || settings.tonemapping) {
            video_reasons.push(TranscodeReason::HdrNotSupported);
        }
    }
    if video.is_interlaced() && !client.interlaced {
        video_reasons.push(TranscodeReason::InterlacedNotSupported);
    }
    let source_height = video.height.and_then(|h| u32::try_from(h).ok()).unwrap_or(0);
    let source_width = video.width.and_then(|w| u32::try_from(w).ok()).unwrap_or(0);
    if request.max_height.is_some_and(|max| source_height > max) {
        video_reasons.push(TranscodeReason::ResolutionAboveRequested);
    }
    let limit = match (input.remote_limit_kbps, request.max_bitrate_kbps) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let source_bitrate = probe.bitrate_kbps.and_then(|b| u32::try_from(b).ok()).unwrap_or(0);
    if limit.is_some_and(|limit| source_bitrate == 0 || source_bitrate > limit) {
        video_reasons.push(TranscodeReason::BitrateAboveLimit);
    }
    if burn_in.is_some() {
        video_reasons.push(TranscodeReason::SubtitleBurnIn);
    }

    // --- audio ------------------------------------------------------------
    let audio_copy = |for_hls: bool| -> (bool, Vec<TranscodeReason>) {
        let Some(audio) = audio else { return (true, vec![]) };
        let codec = audio.codec.as_deref().unwrap_or_default();
        let channels = audio.channels.and_then(|c| u32::try_from(c).ok()).unwrap_or(2);
        let mut why = vec![];
        if !client.audio(audio_capability(codec)) || (for_hls && !fmp4_audio(codec)) {
            why.push(TranscodeReason::AudioCodecNotSupported);
        }
        if channels > client.max_audio_channels {
            why.push(TranscodeReason::AudioChannelsNotSupported);
        }
        if request.downmix != DownmixPreference::Auto && channels > 2 {
            why.push(TranscodeReason::DownmixRequested);
        }
        (why.is_empty(), why)
    };

    let container = container_key(probe);
    let text_subtitle_in_file = subtitle.as_ref().is_some_and(|(track, kind)| {
        *kind == SubtitleKind::Text && matches!(track, TrackRef::Embedded(_)) && burn_in.is_none()
    });

    let method = if !video_reasons.is_empty() {
        reasons.extend(video_reasons.iter().copied());
        PlayMethod::Transcode
    } else {
        let mut direct_reasons = vec![];
        if !client.container(&container) {
            direct_reasons.push(TranscodeReason::ContainerNotSupported);
        }
        let (direct_audio, why) = audio_copy(false);
        if !direct_audio {
            direct_reasons.extend(why);
        }
        if audio.is_some_and(|a| Some(a.index) != probe.first_audio().map(|f| f.index)) {
            direct_reasons.push(TranscodeReason::AudioTrackNotFirst);
        }
        if text_subtitle_in_file {
            direct_reasons.push(TranscodeReason::SubtitleNeedsHls);
        }
        if direct_reasons.is_empty() || client.hls == HlsSupport::None {
            reasons.extend(direct_reasons);
            PlayMethod::DirectPlay
        } else {
            reasons.extend(direct_reasons);
            if !settings.allow_remux {
                reasons.push(TranscodeReason::RemuxDisabled);
                PlayMethod::Transcode
            } else if !fmp4_video(video.codec.as_deref().unwrap_or_default()) {
                reasons.push(TranscodeReason::VideoCodecNotSupported);
                PlayMethod::Transcode
            } else if !input.keyframes_available {
                reasons.push(TranscodeReason::NoKeyframeIndex);
                PlayMethod::Transcode
            } else {
                PlayMethod::Remux
            }
        }
    };

    // --- audio plan -----------------------------------------------------
    let audio_plan = audio.map(|audio| {
        let codec = audio.codec.clone().unwrap_or_default();
        let source_channels = audio.channels.and_then(|c| u32::try_from(c).ok()).unwrap_or(2);
        let copy = match method {
            PlayMethod::DirectPlay => true,
            _ => {
                let (copy, why) = audio_copy(true);
                for reason in why {
                    if !reasons.contains(&reason) {
                        reasons.push(reason);
                    }
                }
                copy
            }
        };
        let action = if copy {
            AudioAction::Copy
        } else {
            let surround = request.downmix == DownmixPreference::Auto
                && client.max_audio_channels >= 6
                && source_channels > 2;
            let channels = if surround { source_channels.min(6) } else { source_channels.min(2) };
            let codec_out = if channels > 2 && client.audio("eac3") {
                AudioCodecOut::Eac3
            } else {
                AudioCodecOut::Aac
            };
            let bitrate_kbps = match (codec_out, channels) {
                (AudioCodecOut::Eac3, _) => 640,
                (AudioCodecOut::Aac, c) if c > 2 => 384,
                _ => settings.audio_bitrate_kbps,
            };
            AudioAction::Encode {
                codec: codec_out,
                bitrate_kbps,
                channels,
                downmix: downmix::choose(
                    request.downmix,
                    DownmixAlgorithm::from_setting(&settings.downmix_algorithm),
                    source_channels,
                    channels,
                    audio.title.as_deref(),
                ),
            }
        };
        AudioPlan { stream_index: audio.index, action, source_codec: codec, source_channels }
    });

    // --- video plan -----------------------------------------------------
    let (video_action, out_w, out_h, out_codec, out_bitrate) = if method == PlayMethod::Transcode {
        let encode = video_encode(input, video, hdr, limit, &audio_plan, burn_in.clone())?;
        let (w, h, codec, bitrate) = (encode.width, encode.height, encode.codec.name(), encode.bitrate_kbps);
        (VideoAction::Encode(Box::new(encode)), w, h, codec.to_owned(), bitrate)
    } else {
        (
            VideoAction::Copy,
            source_width,
            source_height,
            video.codec.clone().unwrap_or_default(),
            source_bitrate,
        )
    };

    let subtitle_plan = subtitle.map(|(track, _)| SubtitlePlan {
        track,
        delivery: if burn_in.is_some() {
            SubtitleDelivery::BurnIn
        } else if method == PlayMethod::DirectPlay {
            SubtitleDelivery::External
        } else {
            SubtitleDelivery::Hls
        },
    });

    Ok(PlaybackPlan {
        method,
        reasons,
        video_index: video.index,
        video: video_action,
        audio: audio_plan,
        subtitle: subtitle_plan,
        output_width: out_w,
        output_height: out_h,
        output_video_codec: out_codec,
        output_bitrate_kbps: out_bitrate,
        source_hdr: hdr,
    })
}

fn subtitle_kind(
    probe: &MediaProbe,
    externals: &[ExternalSubtitle],
    track: TrackRef,
) -> Result<SubtitleKind, &'static str> {
    match track {
        TrackRef::Embedded(index) => probe
            .stream(index)
            .filter(|s| s.kind == StreamKind::Subtitle)
            .and_then(|s| subtitles::kind_of_codec(s.codec.as_deref().unwrap_or_default()))
            .ok_or("subtitle_track_not_found"),
        TrackRef::External(n) => externals.get(n).map(|e| e.kind).ok_or("subtitle_track_not_found"),
    }
}

fn burn_in_source(
    probe: &MediaProbe,
    externals: &[ExternalSubtitle],
    track: TrackRef,
    kind: SubtitleKind,
) -> Option<BurnIn> {
    let source = match track {
        TrackRef::Embedded(index) => BurnInSource::Embedded {
            stream_index: index,
            ordinal: probe
                .streams
                .iter()
                .filter(|s| s.kind == StreamKind::Subtitle)
                .position(|s| s.index == index)?,
        },
        TrackRef::External(n) => {
            let external = externals.get(n)?;
            BurnInSource::External { path: external.path.clone(), format: external.format.clone() }
        }
    };
    Some(BurnIn { source, kind })
}

fn video_encode(
    input: &DecisionInput,
    video: &MediaStream,
    hdr: Option<HdrFormat>,
    limit: Option<u32>,
    audio: &Option<AudioPlan>,
    burn_in: Option<BurnIn>,
) -> Result<VideoEncode, &'static str> {
    let DecisionInput { client, settings, tools, request, .. } = *input;
    let api = hardware_api(settings);
    let encodes = |codec: OutputCodec| {
        let name = match codec {
            OutputCodec::H264 => "h264",
            OutputCodec::Hevc => "hevc",
        };
        api.is_some_and(|api| {
            settings.hardware_encoding
                && settings.hardware_encode_codecs.iter().any(|c| c == name)
                && tools.has_encoder(&hardware_encoder(api, codec))
        })
    };
    let codec = if settings.hevc_encoding && client.video("hevc") && encodes(OutputCodec::Hevc) {
        OutputCodec::Hevc
    } else {
        OutputCodec::H264
    };
    let gpu_encode = encodes(codec);
    let source_codec = video.codec.clone().unwrap_or_default();
    let depth = video.effective_bit_depth();
    let format = video.pixel_format.as_deref().unwrap_or_default();
    let gpu_decode = api.is_some_and(|api| hardware_decodes(api, settings, tools, &source_codec, depth, format));

    let mut max_height = settings.max_output_height;
    if let Some(requested) = request.max_height {
        max_height = max_height.min(requested.max(144));
    }
    if client.max_height > 0 {
        max_height = max_height.min(client.max_height.max(144));
    }
    let source_width = video.width.and_then(|w| u32::try_from(w).ok()).ok_or("resolution_unavailable")?;
    let source_height = video.height.and_then(|h| u32::try_from(h).ok()).ok_or("resolution_unavailable")?;
    let (width, height) = fit_dimensions(source_width, source_height, max_height);

    // Reserve headroom for the container and the audio within a limit.
    let audio_kbps = match audio.as_ref().map(|a| &a.action) {
        Some(AudioAction::Encode { bitrate_kbps, .. }) => *bitrate_kbps,
        Some(AudioAction::Copy) => 256,
        None => 0,
    };
    let bitrate_kbps = match limit {
        Some(limit) => (limit.saturating_mul(90) / 100)
            .saturating_sub(audio_kbps)
            .min(settings.video_bitrate_kbps),
        None => settings.video_bitrate_kbps,
    };
    if bitrate_kbps < 100 {
        return Err("streaming_limit_too_low");
    }

    let keeps_hdr = |format: HdrFormat| {
        codec == OutputCodec::Hevc
            && client.shows_hdr(format)
            && !matches!(format, HdrFormat::DolbyVision { profile: 5 })
    };
    let hdr_passthrough = hdr.is_some_and(keeps_hdr);
    let tonemap = match hdr {
        Some(format) if !hdr_passthrough && settings.tonemapping => {
            select_backend(settings, tools).map(|backend| TonemapPlan {
                algorithm: tonemap::algorithm_name(backend, &settings.tonemapping_algorithm, tools),
                backend,
                peak: settings.tonemapping_peak,
                desat: settings.tonemapping_desat,
                param: settings.tonemapping_param,
                transfer: if format == HdrFormat::Hlg { "arib-std-b67" } else { "smpte2084" },
                dolby_vision: matches!(format, HdrFormat::DolbyVision { .. }),
            })
        }
        _ => None,
    };
    let deinterlace = (video.is_interlaced() && settings.deinterlace_method != "none")
        .then(|| (settings.deinterlace_method.clone(), settings.deinterlace_double_rate));

    Ok(VideoEncode {
        codec,
        width,
        height,
        bitrate_kbps,
        gpu_decode,
        gpu_encode,
        tonemap,
        hdr_passthrough,
        deinterlace,
        burn_in,
        frame_rate: video.frame_rate.filter(|f| f.is_finite() && *f > 1.0).unwrap_or(24.0),
        source_width,
        source_height,
        source_bit_depth: depth,
        source_codec,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::profile::ClientFamily;
    use lisek_core::media_probe::MediaStream;

    pub fn stream(index: i32, kind: StreamKind, codec: &str) -> MediaStream {
        MediaStream {
            index,
            kind,
            codec: Some(codec.into()),
            profile: None,
            language: None,
            title: None,
            is_default: false,
            is_forced: false,
            width: None,
            height: None,
            bit_depth: None,
            pixel_format: None,
            color_transfer: None,
            color_primaries: None,
            color_space: None,
            field_order: None,
            frame_rate: None,
            channels: None,
            channel_layout: None,
            sample_rate: None,
            bitrate_kbps: None,
            dovi_profile: None,
        }
    }

    /// A Blu-ray rip: H.264 1080p, DTS 5.1 + AC-3 lektor, PGS + SRT.
    pub fn bluray_rip() -> MediaProbe {
        let mut video = stream(0, StreamKind::Video, "h264");
        video.width = Some(1920);
        video.height = Some(1080);
        video.pixel_format = Some("yuv420p".into());
        video.frame_rate = Some(23.976);
        let mut dts = stream(1, StreamKind::Audio, "dts");
        dts.channels = Some(6);
        dts.is_default = true;
        let mut lektor = stream(2, StreamKind::Audio, "ac3");
        lektor.channels = Some(6);
        lektor.title = Some("Lektor".into());
        let pgs = stream(3, StreamKind::Subtitle, "hdmv_pgs_subtitle");
        let srt = stream(4, StreamKind::Subtitle, "subrip");
        MediaProbe {
            container: Some("matroska,webm".into()),
            duration_seconds: Some(7200.0),
            bitrate_kbps: Some(30000),
            start_time_seconds: Some(0.0),
            streams: vec![video, dts, lektor, pgs, srt],
            chapters: Vec::new(),
        }
    }

    pub fn hdr_rip() -> MediaProbe {
        let mut probe = bluray_rip();
        let video = &mut probe.streams[0];
        video.codec = Some("hevc".into());
        video.width = Some(3840);
        video.height = Some(2160);
        video.pixel_format = Some("yuv420p10le".into());
        video.color_transfer = Some("smpte2084".into());
        probe
    }

    pub struct Fixture {
        pub probe: MediaProbe,
        pub client: ClientCapabilities,
        pub settings: PlaybackSettings,
        pub tools: FfmpegTools,
        pub request: PlaybackRequest,
        pub externals: Vec<ExternalSubtitle>,
        pub remote: Option<u32>,
        pub keyframes: bool,
    }

    impl Fixture {
        pub fn new(probe: MediaProbe, family: ClientFamily) -> Self {
            Self {
                probe,
                client: ClientCapabilities::preset(family),
                settings: PlaybackSettings::default(),
                tools: FfmpegTools::default(),
                request: PlaybackRequest::default(),
                externals: vec![],
                remote: None,
                keyframes: true,
            }
        }
        pub fn decide(&self) -> Result<PlaybackPlan, &'static str> {
            decide(&DecisionInput {
                probe: &self.probe,
                externals: &self.externals,
                client: &self.client,
                settings: &self.settings,
                tools: &self.tools,
                request: &self.request,
                remote_limit_kbps: self.remote,
                keyframes_available: self.keyframes,
            })
        }
    }

    fn nvidia_tools() -> FfmpegTools {
        FfmpegTools::parse(
            "ffmpeg version 7.1.2-Jellyfin",
            " ... tonemap_cuda V->V x\n ... scale_cuda V->V x\n ... libplacebo N->V x\n ... zscale V->V x\n ... tonemap V->V x\n",
            " V....D h264_nvenc x\n V....D hevc_nvenc x\n V....D libx264 x\n A....D aac x\n A....D eac3 x\n",
            "Hardware acceleration methods:\ncuda\n",
        )
    }

    #[test]
    fn mkv_h264_with_dts_remuxes_and_reencodes_only_the_audio() {
        let f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        let plan = f.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Remux);
        assert_eq!(plan.video, VideoAction::Copy);
        assert!(plan.reasons.contains(&TranscodeReason::ContainerNotSupported));
        assert!(plan.reasons.contains(&TranscodeReason::AudioCodecNotSupported));
        let audio = plan.audio.unwrap();
        assert_eq!(audio.stream_index, 1, "the default track");
        assert_eq!(
            audio.action,
            AudioAction::Encode {
                codec: AudioCodecOut::Aac,
                bitrate_kbps: 128,
                channels: 2,
                downmix: Some(DownmixAlgorithm::Standard)
            }
        );
    }

    #[test]
    fn lektor_track_downmixes_for_dialogue_and_surround_clients_get_eac3() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.request.audio_stream_index = Some(2);
        let plan = f.decide().unwrap();
        let Some(AudioAction::Encode { downmix, .. }) = plan.audio.map(|a| a.action) else { panic!() };
        assert_eq!(downmix, Some(DownmixAlgorithm::Dialogue));

        // TVs take MKV and AC-3 5.1, but a URL cannot pick the second
        // audio track: remux with the audio copied.
        let mut tv = Fixture::new(bluray_rip(), ClientFamily::Tv);
        tv.request.audio_stream_index = Some(2);
        let plan = tv.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Remux);
        assert_eq!(plan.reasons, [TranscodeReason::AudioTrackNotFirst]);
        assert_eq!(plan.audio.unwrap().action, AudioAction::Copy);
        // DTS (the default track) is not in the TV preset: E-AC-3 5.1.
        tv.request.audio_stream_index = None;
        let plan = tv.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Remux);
        assert_eq!(
            plan.audio.unwrap().action,
            AudioAction::Encode { codec: AudioCodecOut::Eac3, bitrate_kbps: 640, channels: 6, downmix: None }
        );
    }

    #[test]
    fn mp4_h264_aac_plays_directly() {
        let mut probe = bluray_rip();
        probe.container = Some("mov,mp4,m4a,3gp,3g2,mj2".into());
        probe.streams[1].codec = Some("aac".into());
        probe.streams[1].channels = Some(2);
        probe.streams.truncate(2);
        let plan = Fixture::new(probe, ClientFamily::Firefox).decide().unwrap();
        assert_eq!(plan.method, PlayMethod::DirectPlay);
        assert!(plan.reasons.is_empty());
        assert_eq!(plan.audio.unwrap().action, AudioAction::Copy);
    }

    #[test]
    fn pgs_subtitle_is_burnt_in_and_forces_a_transcode() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.request.subtitle = Some(TrackRef::Embedded(3));
        let plan = f.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Transcode);
        assert_eq!(plan.subtitle.as_ref().unwrap().delivery, SubtitleDelivery::BurnIn);
        let VideoAction::Encode(encode) = &plan.video else { panic!() };
        assert_eq!(
            encode.burn_in.as_ref().unwrap().source,
            BurnInSource::Embedded { stream_index: 3, ordinal: 0 }
        );
        assert_eq!((encode.width, encode.height), (1280, 720), "default max_output_height");
    }

    #[test]
    fn embedded_text_subtitle_moves_direct_play_to_remux_for_hls_delivery() {
        let mut probe = bluray_rip();
        probe.container = Some("mov,mp4".into());
        probe.streams[1].codec = Some("aac".into());
        probe.streams[1].channels = Some(2);
        let mut f = Fixture::new(probe, ClientFamily::Chrome);
        f.request.subtitle = Some(TrackRef::Embedded(4));
        let plan = f.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Remux);
        assert_eq!(plan.reasons, [TranscodeReason::SubtitleNeedsHls]);
        assert_eq!(plan.subtitle.unwrap().delivery, SubtitleDelivery::Hls);
        f.request.burn_subtitle = true;
        assert_eq!(f.decide().unwrap().method, PlayMethod::Transcode);
    }

    #[test]
    fn hdr_on_an_sdr_screen_is_tone_mapped_on_the_gpu() {
        let mut f = Fixture::new(hdr_rip(), ClientFamily::Chrome);
        f.client.video_codecs.push("hevc_10bit".into());
        f.settings.tonemapping = true;
        f.settings.hardware_acceleration = "nvenc".into();
        f.tools = nvidia_tools();
        let plan = f.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Transcode);
        assert_eq!(plan.reasons[0], TranscodeReason::HdrNotSupported);
        let VideoAction::Encode(encode) = &plan.video else { panic!() };
        let tonemap = encode.tonemap.as_ref().unwrap();
        assert_eq!(tonemap.backend, TonemapBackend::Cuda);
        assert_eq!(tonemap.algorithm, "bt2390");
        assert!(encode.gpu_decode && encode.gpu_encode);

        // An HDR screen gets the original stream.
        f.client.hdr_formats = vec!["hdr10".into()];
        assert_eq!(f.decide().unwrap().method, PlayMethod::Remux);
        // Without tone mapping enabled, HDR is passed on and the client copes.
        f.client.hdr_formats.clear();
        f.settings.tonemapping = false;
        assert_eq!(f.decide().unwrap().method, PlayMethod::Remux);
    }

    #[test]
    fn dolby_vision_profile_5_is_never_copied_to_non_dolby_vision_clients() {
        let mut probe = hdr_rip();
        probe.streams[0].dovi_profile = Some(5);
        let mut f = Fixture::new(probe, ClientFamily::Chrome);
        f.client.video_codecs.push("hevc_10bit".into());
        f.client.hdr_formats = vec!["hdr10".into()];
        let plan = f.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Transcode);
        assert!(plan.reasons.contains(&TranscodeReason::HdrNotSupported));
    }

    #[test]
    fn remote_limit_transcodes_within_budget() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.remote = Some(4000);
        let plan = f.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Transcode);
        let VideoAction::Encode(encode) = &plan.video else { panic!() };
        assert_eq!(encode.bitrate_kbps, 2500, "capped by video_bitrate_kbps");
        f.remote = Some(1000);
        let VideoAction::Encode(encode) = f.decide().unwrap().video else { panic!() };
        assert_eq!(encode.bitrate_kbps, 900 - 128);
        f.remote = Some(200);
        assert_eq!(f.decide().unwrap_err(), "streaming_limit_too_low");
    }

    #[test]
    fn copy_needs_a_keyframe_index_and_remux_can_be_disabled() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.keyframes = false;
        let plan = f.decide().unwrap();
        assert_eq!(plan.method, PlayMethod::Transcode);
        assert!(plan.reasons.contains(&TranscodeReason::NoKeyframeIndex));
        f.keyframes = true;
        f.settings.allow_remux = false;
        assert!(f.decide().unwrap().reasons.contains(&TranscodeReason::RemuxDisabled));
    }

    #[test]
    fn hi10p_and_interlaced_video_are_transcoded_for_browsers() {
        let mut probe = bluray_rip();
        probe.streams[0].pixel_format = Some("yuv420p10le".into());
        let plan = Fixture::new(probe, ClientFamily::Chrome).decide().unwrap();
        assert_eq!(plan.reasons[0], TranscodeReason::VideoBitDepthNotSupported);
        let mut probe = bluray_rip();
        probe.streams[0].field_order = Some("tt".into());
        let plan = Fixture::new(probe, ClientFamily::Chrome).decide().unwrap();
        let VideoAction::Encode(encode) = plan.video else { panic!() };
        assert_eq!(encode.deinterlace, Some(("yadif".into(), false)));
    }

    #[test]
    fn unknown_tracks_are_rejected() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.request.audio_stream_index = Some(0);
        assert_eq!(f.decide().unwrap_err(), "audio_stream_not_found");
        f.request.audio_stream_index = None;
        f.request.subtitle = Some(TrackRef::External(0));
        assert_eq!(f.decide().unwrap_err(), "subtitle_track_not_found");
    }

    #[test]
    fn fits_without_upscaling() {
        assert_eq!(fit_dimensions(3840, 2160, 1080), (1920, 1080));
        assert_eq!(fit_dimensions(1920, 800, 720), (1280, 534));
        assert_eq!(fit_dimensions(720, 576, 1080), (720, 576));
        assert_eq!(fit_dimensions(1080, 1920, 720), (406, 720), "portrait");
    }
}
