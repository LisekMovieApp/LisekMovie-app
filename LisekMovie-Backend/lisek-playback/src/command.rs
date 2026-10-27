//! The FFmpeg command line of one HLS run: from a [`PlaybackPlan`] and a
//! start position to fragmented MP4 on stdout (cut into HLS segments by
//! [`crate::session`]), plus one WebVTT file per text subtitle stream.
//!
//! Timestamps: `-copyts` keeps the file's own timeline, so filters
//! (`subtitles`, forced keyframes) see real times and every run, whatever
//! its start, produces the same timestamps for the same frame. A constant
//! `-output_ts_offset` keeps decode times positive (B-frames, AAC priming);
//! `frag_discont` + `-use_editlist 0` make the MP4 muxer write those
//! timestamps as they are instead of rebasing each run to zero.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use lisek_core::playback_settings::PlaybackSettings;

use crate::decision::{
    AudioAction, AudioCodecOut, BurnInSource, OutputCodec, PlaybackPlan, VideoAction, VideoEncode,
    hardware_api, hardware_encoder,
};
use crate::filters::{Frames, VideoChain};
use crate::subtitles::{self, SubtitleKind};
use crate::tonemap;
use crate::transcode::FfmpegTools;

/// Added to every output timestamp (seconds).
pub const OUTPUT_TS_OFFSET: f64 = 10.0;

pub struct RunArgs<'a> {
    pub input: &'a Path,
    pub plan: &'a PlaybackPlan,
    pub settings: &'a PlaybackSettings,
    pub tools: &'a FfmpegTools,
    /// Playlist time to start at, seconds from the file's start.
    pub start: f64,
    /// The file's start time (ffprobe `start_time`): playlist time zero.
    pub origin: f64,
    /// Forced keyframe interval when the video is encoded.
    pub segment_seconds: f64,
    /// Text subtitle streams to export: (ffprobe index, output file).
    pub subtitle_outputs: &'a [(i32, PathBuf)],
    /// `-sub_charenc` for an external text subtitle that is burnt in.
    pub external_charset: Option<&'a str>,
}

fn os(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn nvenc_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" | "superfast" => "p1",
        "veryfast" | "faster" => "p2",
        "fast" => "p3",
        "medium" => "p4",
        "slow" => "p5",
        "slower" => "p6",
        _ => "p7",
    }
}

/// Quick Sync presets are named like x264's, without the two fastest.
fn qsv_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" | "superfast" | "veryfast" => "veryfast",
        "faster" => "faster",
        "fast" => "fast",
        "medium" => "medium",
        "slow" => "slow",
        "slower" => "slower",
        _ => "veryslow",
    }
}

/// The video filter graph (for `-filter_complex`, output label `[vout]`)
/// and whether it needs Vulkan or OpenCL devices.
pub fn video_graph(
    plan: &PlaybackPlan,
    encode: &VideoEncode,
    settings: &PlaybackSettings,
    tools: &FfmpegTools,
    input: &Path,
    external_charset: Option<&str>,
) -> (String, VideoChain) {
    let ten_bit = encode.source_bit_depth > 8;
    let sw = if ten_bit { "p010le" } else { "nv12" };
    let api = hardware_api(settings);
    let frames = match (encode.gpu_decode, api) {
        (true, Some("qsv")) => Frames::Qsv { sw },
        (true, Some("vaapi")) => Frames::Vaapi { sw },
        (true, _) => Frames::Cuda { sw },
        (false, _) => Frames::Cpu,
    };
    let mut chain = VideoChain::new(frames, ten_bit, settings.hardware_device_index);
    let (w, h) = (encode.width, encode.height);

    if let Some((method, double_rate)) = &encode.deinterlace {
        let mode = if *double_rate { "send_field" } else { "send_frame" };
        let gpu_filter = format!("{method}_cuda");
        if chain.frames().is_cuda() && tools.has_filter(&gpu_filter) {
            chain.cuda(format!("{gpu_filter}=mode={mode}:parity=auto:deint=interlaced"), None);
        } else {
            chain.cpu(format!("{method}=mode={mode}:parity=auto:deint=interlaced"));
        }
    }

    if let Some(tonemap_plan) = &encode.tonemap {
        tonemap::apply(&mut chain, tonemap_plan, w, h);
        chain.set_eight_bit();
    } else if encode.hdr_passthrough {
        if chain.frames().is_cuda() && tools.has_filter("scale_cuda") {
            chain.cuda(format!("scale_cuda=w={w}:h={h}:format=p010le"), Some("p010le"));
        } else if chain.frames().is_qsv() && tools.has_filter("scale_qsv") {
            chain.qsv(format!("scale_qsv=w={w}:h={h}:format=p010"), Some("p010le"));
        } else if chain.frames().is_vaapi() && tools.has_filter("scale_vaapi") {
            chain.vaapi(format!("scale_vaapi=w={w}:h={h}:format=p010"), Some("p010le"));
        } else {
            chain.cpu(format!("scale=w={w}:h={h}:flags=bicubic"));
            chain.cpu("format=yuv420p10le");
        }
    } else if chain.frames().is_cuda() && tools.has_filter("scale_cuda") {
        chain.cuda(format!("scale_cuda=w={w}:h={h}:interp_algo=bicubic:format=nv12"), Some("nv12"));
        chain.set_eight_bit();
    } else if chain.frames().is_qsv() && tools.has_filter("scale_qsv") {
        chain.qsv(format!("scale_qsv=w={w}:h={h}:format=nv12"), Some("nv12"));
        chain.set_eight_bit();
    } else if chain.frames().is_vaapi() && tools.has_filter("scale_vaapi") {
        chain.vaapi(format!("scale_vaapi=w={w}:h={h}:format=nv12"), Some("nv12"));
        chain.set_eight_bit();
    } else {
        chain.cpu(format!("scale=w={w}:h={h}:flags=bicubic"));
        chain.cpu("format=yuv420p");
        chain.set_eight_bit();
    }

    let video_in = format!("[0:{}]", plan.video_index);
    let mut overlay_input = None;
    if let Some(burn_in) = &encode.burn_in {
        match (&burn_in.source, burn_in.kind) {
            (BurnInSource::Embedded { ordinal, .. }, SubtitleKind::Text) => {
                let file = subtitles::escape_filter_value(&input.to_string_lossy());
                chain.cpu(format!("subtitles=filename={file}:si={ordinal}"));
            }
            (BurnInSource::External { path, .. }, SubtitleKind::Text) => {
                let file = subtitles::escape_filter_value(&path.to_string_lossy());
                let charenc = external_charset
                    .map(|c| format!(":charenc={c}"))
                    .unwrap_or_default();
                chain.cpu(format!("subtitles=filename={file}{charenc}"));
            }
            (BurnInSource::Embedded { stream_index, .. }, SubtitleKind::Image) => {
                overlay_input = Some(format!("[0:{stream_index}]"));
            }
            (BurnInSource::External { .. }, SubtitleKind::Image) => {
                overlay_input = Some("[1:s:0]".to_owned());
            }
        }
    }

    // The encoder's input: NVENC takes CUDA or system frames, Quick Sync
    // and VA-API encoders only their own surfaces, software encoders only
    // system memory.
    let finish = |chain: &mut VideoChain| match (encode.gpu_encode, api) {
        (true, Some("qsv")) => chain.to_qsv(),
        (true, Some("vaapi")) => chain.to_vaapi(),
        (true, _) if chain.frames().is_cuda() => {}
        _ => {
            chain.to_cpu();
            chain.cpu(if encode.hdr_passthrough { "format=yuv420p10le" } else { "format=yuv420p" });
        }
    };

    let graph = match overlay_input {
        None => {
            finish(&mut chain);
            format!("{video_in}{}[vout]", chain.to_filter_string())
        }
        Some(sub_in) => {
            chain.to_cpu();
            let pre = chain.to_filter_string();
            let mut post = VideoChain::new(Frames::Cpu, encode.hdr_passthrough, settings.hardware_device_index);
            finish(&mut post);
            chain.uses_qsv |= post.uses_qsv;
            chain.uses_vaapi |= post.uses_vaapi;
            format!(
                "{video_in}{pre}[base];{sub_in}scale=w={w}:h={h}[sub];\
                 [base][sub]overlay=eof_action=pass,{}[vout]",
                post.to_filter_string()
            )
        }
    };
    (graph, chain)
}

/// `-init_hw_device` for Quick Sync / VA-API (Faza 14). Linux: the render
/// node opens VA-API, Quick Sync is derived from it (as Intel's media
/// driver expects). Windows: Quick Sync on a DirectX 11 adapter, or the
/// first Intel one with `auto`.
pub fn intel_device_args(api: &str, device: &str) -> Vec<OsString> {
    let windows_style = !device.starts_with('/');
    match (api, windows_style) {
        ("qsv", true) if device.is_empty() || device == "auto" => os(&[
            "-init_hw_device", "qsv=qs:hw_any,child_device_type=d3d11va", "-filter_hw_device", "qs",
        ]),
        ("qsv", true) => os(&[
            "-init_hw_device", &format!("d3d11va=dx:{device}"), "-init_hw_device", "qsv=qs@dx",
            "-filter_hw_device", "qs",
        ]),
        ("qsv", false) => os(&[
            "-init_hw_device", &format!("vaapi=va:{device}"), "-init_hw_device", "qsv=qs@va",
            "-filter_hw_device", "qs",
        ]),
        (_, _) => {
            let node = if device.is_empty() || windows_style { "/dev/dri/renderD128" } else { device };
            os(&["-init_hw_device", &format!("vaapi=va:{node}"), "-filter_hw_device", "va"])
        }
    }
}

/// Arguments for `ffmpeg` (without the binary).
pub fn hls_run_args(a: &RunArgs) -> Vec<OsString> {
    let plan = a.plan;
    let settings = a.settings;
    let device = settings.hardware_device_index;
    let mut args = os(&["-hide_banner", "-loglevel", "warning", "-nostdin", "-y", "-copyts"]);

    let encode = match &plan.video {
        VideoAction::Encode(encode) => Some(encode.as_ref()),
        VideoAction::Copy => None,
    };
    let graph = encode.map(|e| video_graph(plan, e, settings, a.tools, a.input, a.external_charset));

    if let Some((_, chain)) = &graph
        && let Some(api @ ("qsv" | "vaapi")) = hardware_api(settings)
    {
        let e = encode.expect("graph implies encode");
        if e.gpu_decode || e.gpu_encode || chain.uses_qsv || chain.uses_vaapi {
            args.extend(intel_device_args(api, &settings.hardware_device_path));
        }
        if e.gpu_decode {
            if api == "qsv" {
                args.extend(os(&[
                    "-hwaccel", "qsv", "-hwaccel_device", "qs", "-hwaccel_output_format", "qsv",
                    "-c:v", &format!("{}_qsv", crate::decision::qsv_decoder_codec(&e.source_codec)),
                ]));
            } else {
                args.extend(os(&[
                    "-hwaccel", "vaapi", "-hwaccel_device", "va", "-hwaccel_output_format", "vaapi",
                ]));
            }
        }
    } else if let Some((_, chain)) = &graph {
        let e = encode.expect("graph implies encode");
        let cuda = e.gpu_decode || chain.uses_cuda;
        if cuda {
            args.extend(os(&["-init_hw_device", &format!("cuda=cu:{device}")]));
        }
        if chain.uses_vulkan {
            let vk = if cuda { "vulkan=vk@cu".to_owned() } else { format!("vulkan=vk:{device}") };
            args.extend(os(&["-init_hw_device", &vk, "-filter_hw_device", "vk"]));
        } else if chain.uses_opencl {
            args.extend(os(&["-init_hw_device", "opencl=ocl", "-filter_hw_device", "ocl"]));
        }
        if e.gpu_decode {
            args.extend(os(&[
                "-hwaccel", "cuda", "-hwaccel_device", "cu", "-hwaccel_output_format", "cuda",
            ]));
        }
    }

    if a.start > 0.0 {
        args.extend(os(&["-ss", &format!("{:.3}", a.start)]));
    }
    args.push("-i".into());
    args.push(a.input.into());

    let external_image = encode.and_then(|e| e.burn_in.as_ref()).and_then(|b| match (&b.source, b.kind) {
        (BurnInSource::External { path, .. }, SubtitleKind::Image) => Some(path.clone()),
        _ => None,
    });
    if let Some(path) = external_image {
        // External subtitle files count from zero; shift them onto the
        // video's timeline (and the seek) like the main input.
        let offset = a.origin + a.start;
        if offset != 0.0 {
            args.extend(os(&["-itsoffset", &format!("{offset:.3}")]));
        }
        if a.start > 0.0 {
            args.extend(os(&["-ss", "0"]));
        }
        args.push("-i".into());
        args.push(path.into());
    }

    match (&plan.video, &graph) {
        (VideoAction::Copy, _) => {
            args.extend(os(&["-map", &format!("0:{}", plan.video_index), "-c:v", "copy"]));
            if plan.output_video_codec == "hevc" {
                // Apple players (and Chrome on macOS) want `hvc1`.
                args.extend(os(&["-tag:v", "hvc1"]));
            }
        }
        (VideoAction::Encode(e), Some((graph, _))) => {
            args.extend(os(&["-filter_complex", graph, "-map", "[vout]"]));
            args.extend(video_encoder_args(e, settings, a.segment_seconds));
        }
        (VideoAction::Encode(_), None) => unreachable!(),
    }

    if let Some(audio) = &plan.audio {
        args.extend(os(&["-map", &format!("0:{}", audio.stream_index)]));
        match &audio.action {
            AudioAction::Copy => {
                args.extend(os(&["-c:a", "copy"]));
                if matches!(audio.source_codec.as_str(), "flac" | "opus") {
                    // Still marked experimental in MP4 by some builds.
                    args.extend(os(&["-strict", "-2"]));
                }
            }
            AudioAction::Encode { codec, bitrate_kbps, channels, downmix } => {
                let encoder = match codec {
                    AudioCodecOut::Aac => "aac",
                    AudioCodecOut::Eac3 => "eac3",
                };
                args.extend(os(&[
                    "-c:a", encoder, "-b:a", &format!("{bitrate_kbps}k"), "-ac", &channels.to_string(),
                ]));
                if let Some(downmix) = downmix {
                    args.extend(os(&["-af", &downmix.filter(settings.downmix_audio_boost)]));
                }
            }
        }
    }

    args.extend(os(&[
        "-map_metadata", "-1", "-map_chapters", "-1",
        "-max_muxing_queue_size", &settings.max_muxing_queue_size.to_string(),
        "-avoid_negative_ts", "disabled",
        "-output_ts_offset", &format!("{OUTPUT_TS_OFFSET}"),
        "-use_editlist", "0",
        "-movflags", "+frag_keyframe+empty_moov+default_base_moof+frag_discont",
        "-f", "mp4", "pipe:1",
    ]));

    for (index, path) in a.subtitle_outputs {
        args.extend(os(&[
            "-map", &format!("0:{index}"), "-c:s", "webvtt", "-flush_packets", "1", "-f", "webvtt",
        ]));
        args.push(path.into());
    }
    args
}

fn video_encoder_args(e: &VideoEncode, settings: &PlaybackSettings, segment_seconds: f64) -> Vec<OsString> {
    let rate = if e.deinterlace.as_ref().is_some_and(|(_, double)| *double) {
        e.frame_rate * 2.0
    } else {
        e.frame_rate
    };
    let gop = ((segment_seconds * rate).round() as u32).max(1).to_string();
    let bitrate = format!("{}k", e.bitrate_kbps);
    let bufsize = format!("{}k", e.bitrate_kbps.saturating_mul(2));
    let device = settings.hardware_device_index.to_string();
    let profile = match (e.codec, e.hdr_passthrough) {
        (OutputCodec::H264, _) => "high",
        (OutputCodec::Hevc, true) => "main10",
        (OutputCodec::Hevc, false) => "main",
    };
    let mut args = if e.gpu_encode && hardware_api(settings) == Some("qsv") {
        os(&[
            "-c:v", &hardware_encoder("qsv", e.codec), "-preset", qsv_preset(&settings.encoder_preset),
            "-b:v", &bitrate, "-maxrate", &bitrate, "-bufsize", &bufsize, "-profile:v", profile,
            "-g", &gop, "-forced_idr", "1", "-look_ahead", "0",
        ])
    } else if e.gpu_encode && hardware_api(settings) == Some("vaapi") {
        os(&[
            "-c:v", &hardware_encoder("vaapi", e.codec), "-rc_mode", "VBR", "-b:v", &bitrate,
            "-maxrate", &bitrate, "-bufsize", &bufsize, "-profile:v", profile, "-g", &gop,
        ])
    } else if e.gpu_encode {
        os(&[
            "-c:v", &hardware_encoder("nvenc", e.codec), "-gpu", &device, "-preset",
            nvenc_preset(&settings.encoder_preset), "-rc", "vbr", "-b:v", &bitrate, "-maxrate",
            &bitrate, "-bufsize", &bufsize, "-profile:v", profile, "-g", &gop, "-forced-idr", "1",
            "-no-scenecut", "1",
        ])
    } else {
        let mut args = match e.codec {
            OutputCodec::H264 => os(&["-c:v", "libx264", "-profile:v", "high"]),
            OutputCodec::Hevc => os(&["-c:v", "libx265"]),
        };
        args.extend(os(&[
            "-preset", &settings.encoder_preset, "-crf", &settings.h264_crf.to_string(),
            "-maxrate", &bitrate, "-bufsize", &bufsize,
            "-g", &gop, "-keyint_min", &gop, "-sc_threshold", "0",
        ]));
        if settings.encoding_threads > 0 {
            args.extend(os(&["-threads", &settings.encoding_threads.to_string()]));
        }
        args
    };
    if e.codec == OutputCodec::Hevc {
        args.extend(os(&["-tag:v", "hvc1"]));
    }
    // Relative to the run's first frame, which sits on a segment boundary.
    args.extend(os(&["-force_key_frames:v", &format!("expr:gte(t,n_forced*{segment_seconds})")]));
    if e.tonemap.is_some() {
        args.extend(os(&[
            "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709",
            "-color_range", "tv",
        ]));
    } else if e.hdr_passthrough {
        let transfer = e.tonemap.as_ref().map_or("smpte2084", |t| t.transfer);
        args.extend(os(&[
            "-color_primaries", "bt2020", "-color_trc", transfer, "-colorspace", "bt2020nc",
        ]));
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decision::tests::{bluray_rip, hdr_rip, Fixture};
    use crate::profile::ClientFamily;
    use crate::subtitles::TrackRef;

    fn args_of(f: &Fixture, start: f64, subs: &[(i32, PathBuf)]) -> Vec<String> {
        let plan = f.decide().unwrap();
        hls_run_args(&RunArgs {
            input: Path::new("/media/Film: Część 1.mkv"),
            plan: &plan,
            settings: &f.settings,
            tools: &f.tools,
            start,
            origin: 0.0,
            segment_seconds: 4.0,
            subtitle_outputs: subs,
            external_charset: None,
        })
        .into_iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect()
    }

    fn value(args: &[String], key: &str) -> String {
        args[args.iter().position(|a| a == key).unwrap_or_else(|| panic!("{key} missing")) + 1].clone()
    }

    #[test]
    fn remux_copies_video_and_encodes_only_audio() {
        let f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        let args = args_of(&f, 600.0, &[(4, PathBuf::from("/tmp/s4.vtt"))]);
        assert!(!args.contains(&"-re".to_owned()), "no realtime pacing");
        assert_eq!(value(&args, "-ss"), "600.000");
        assert_eq!(value(&args, "-c:v"), "copy");
        assert_eq!(value(&args, "-c:a"), "aac");
        assert_eq!(value(&args, "-ac"), "2");
        assert!(value(&args, "-af").starts_with("aresample=ochl=stereo"));
        assert!(args.contains(&"-copyts".to_owned()));
        assert_eq!(value(&args, "-movflags"), "+frag_keyframe+empty_moov+default_base_moof+frag_discont");
        assert_eq!(args[args.len() - 1], "/tmp/s4.vtt");
        assert_eq!(value(&args, "-f"), "mp4");
        assert!(!args.contains(&"-filter_complex".to_owned()));
    }

    #[test]
    fn nvidia_hdr_transcode_stays_on_the_gpu() {
        let mut f = Fixture::new(hdr_rip(), ClientFamily::Chrome);
        f.client.video_codecs.push("hevc_10bit".into());
        f.settings.tonemapping = true;
        f.settings.hardware_acceleration = "nvenc".into();
        f.settings.max_output_height = 1080;
        f.settings.hardware_device_index = 1;
        f.tools = FfmpegTools::parse(
            "ffmpeg version 7.1-Jellyfin",
            " ... tonemap_cuda V->V x\n ... scale_cuda V->V x\n",
            " V....D h264_nvenc x\n A....D aac x\n",
            "Hardware acceleration methods:\ncuda\n",
        );
        let args = args_of(&f, 0.0, &[]);
        assert_eq!(value(&args, "-init_hw_device"), "cuda=cu:1");
        assert_eq!(value(&args, "-hwaccel_output_format"), "cuda");
        assert_eq!(
            value(&args, "-filter_complex"),
            "[0:0]setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc,\
             scale_cuda=w=1920:h=1080:interp_algo=bicubic,\
             tonemap_cuda=format=nv12:p=bt709:t=bt709:m=bt709:tonemap=bt2390:peak=100:desat=0[vout]"
        );
        assert_eq!(value(&args, "-c:v"), "h264_nvenc");
        assert_eq!(value(&args, "-gpu"), "1");
        assert_eq!(value(&args, "-g"), "96");
        assert_eq!(value(&args, "-force_key_frames:v"), "expr:gte(t,n_forced*4)");
        assert_eq!(value(&args, "-color_trc"), "bt709");
        assert!(!args.contains(&"-ss".to_owned()));
    }

    fn intel_tools() -> FfmpegTools {
        FfmpegTools::parse(
            "ffmpeg version 7.1",
            " ... scale_qsv V->V x\n ... scale_vaapi V->V x\n ... zscale V->V x\n ... tonemap V->V x\n",
            " V....D h264_qsv x\n V....D hevc_qsv x\n V....D h264_vaapi x\n V....D libx264 x\n A....D aac x\n",
            "Hardware acceleration methods:\nvaapi\nqsv\n",
        )
        .with_decoders(" V....D h264_qsv H.264 (Intel Quick Sync Video)\n V....D hevc_qsv x\n V....D h264 x\n")
    }

    #[test]
    fn quick_sync_decodes_scales_and_encodes_on_the_intel_gpu() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.request.force_transcode = true;
        f.settings.hardware_acceleration = "qsv".into();
        f.settings.hardware_device_path = "/dev/dri/renderD128".into();
        f.tools = intel_tools();
        let plan = f.decide().unwrap();
        assert!(plan.uses_gpu());
        let args = args_of(&f, 0.0, &[]);
        let joined = args.join(" ");
        assert!(
            joined.starts_with(
                "-hide_banner -loglevel warning -nostdin -y -copyts \
                 -init_hw_device vaapi=va:/dev/dri/renderD128 -init_hw_device qsv=qs@va -filter_hw_device qs \
                 -hwaccel qsv -hwaccel_device qs -hwaccel_output_format qsv -c:v h264_qsv -i "
            ),
            "{joined}"
        );
        assert_eq!(value(&args, "-filter_complex"), "[0:0]scale_qsv=w=1280:h=720:format=nv12[vout]");
        let encoder = args.iter().rposition(|a| a == "-c:v").unwrap();
        assert_eq!(args[encoder + 1], "h264_qsv");
        assert_eq!(value(&args, "-forced_idr"), "1");
        assert_eq!(value(&args, "-preset"), "veryfast");
        assert!(!args.contains(&"-gpu".to_owned()));
    }

    #[test]
    fn vaapi_burns_text_subtitles_on_the_cpu_between_gpu_steps() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.request.subtitle = Some(TrackRef::Embedded(4));
        f.request.burn_subtitle = true;
        f.settings.hardware_acceleration = "vaapi".into();
        f.settings.hardware_device_path = "/dev/dri/renderD129".into();
        f.tools = intel_tools();
        let args = args_of(&f, 0.0, &[]);
        assert_eq!(value(&args, "-init_hw_device"), "vaapi=va:/dev/dri/renderD129");
        assert_eq!(value(&args, "-filter_hw_device"), "va");
        assert_eq!(value(&args, "-hwaccel"), "vaapi");
        assert_eq!(
            value(&args, "-filter_complex"),
            "[0:0]scale_vaapi=w=1280:h=720:format=nv12,hwdownload,format=nv12,\
             subtitles=filename=/media/Film\\\\: Część 1.mkv:si=1,format=nv12,hwupload[vout]"
        );
        let encoder = args.iter().rposition(|a| a == "-c:v").unwrap();
        assert_eq!(args[encoder + 1], "h264_vaapi");
        assert_eq!(value(&args, "-rc_mode"), "VBR");
    }

    #[test]
    fn quick_sync_hdr_is_tone_mapped_on_the_cpu_and_encoded_on_the_gpu() {
        let mut f = Fixture::new(hdr_rip(), ClientFamily::Chrome);
        f.settings.tonemapping = true;
        f.settings.tonemapping_backend = "libplacebo".into();
        f.settings.hardware_acceleration = "qsv".into();
        f.settings.hardware_device_path = "auto".into();
        f.tools = intel_tools();
        let args = args_of(&f, 0.0, &[]);
        assert_eq!(value(&args, "-init_hw_device"), "qsv=qs:hw_any,child_device_type=d3d11va");
        let graph = value(&args, "-filter_complex");
        assert!(graph.contains("hwdownload,format=p010le"), "HEVC decoded by Quick Sync: {graph}");
        assert!(graph.contains("zscale") || graph.contains("tonemap="), "{graph}");
        assert!(graph.ends_with("format=nv12,hwupload=extra_hw_frames=64[vout]"), "{graph}");
        assert!(!graph.contains("libplacebo"), "no Vulkan on an Intel session: {graph}");
        let encoder = args.iter().rposition(|a| a == "-c:v").unwrap();
        assert_eq!(args[encoder + 1], "h264_qsv");
    }

    #[test]
    fn codec_limits_of_a_pool_device_send_work_to_the_cpu() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.request.force_transcode = true;
        f.settings.hardware_acceleration = "qsv".into();
        f.settings.hardware_device_path = "/dev/dri/renderD128".into();
        f.settings.hardware_decoding_codecs = vec![];
        f.settings.hardware_encode_codecs = vec!["hevc".into()];
        f.tools = intel_tools();
        let plan = f.decide().unwrap();
        assert!(!plan.uses_gpu(), "no H.264 encoding and no decoding on this device");
        let args = args_of(&f, 0.0, &[]);
        assert!(!args.contains(&"-init_hw_device".to_owned()));
        let encoder = args.iter().rposition(|a| a == "-c:v").unwrap();
        assert_eq!(args[encoder + 1], "libx264");
    }

    #[test]
    fn libplacebo_round_trips_through_vulkan() {
        let mut f = Fixture::new(hdr_rip(), ClientFamily::Chrome);
        f.client.video_codecs.push("hevc_10bit".into());
        f.settings.tonemapping = true;
        f.settings.tonemapping_backend = "libplacebo".into();
        f.tools = FfmpegTools::parse("ffmpeg version 6.1", " ..C libplacebo N->V x\n", " V....D libx264 x\n", "");
        let args = args_of(&f, 0.0, &[]);
        assert_eq!(value(&args, "-init_hw_device"), "vulkan=vk:0");
        assert_eq!(value(&args, "-filter_hw_device"), "vk");
        let graph = value(&args, "-filter_complex");
        assert!(graph.contains("hwupload,libplacebo=w=1280:h=720:format=yuv420p"), "{graph}");
        assert!(graph.contains("tonemapping=bt.2390"));
        assert!(graph.ends_with("hwdownload,format=yuv420p,format=yuv420p[vout]"), "{graph}");
        assert_eq!(value(&args, "-c:v"), "libx264");
        assert_eq!(value(&args, "-sc_threshold"), "0");
    }

    #[test]
    fn pgs_is_overlaid_and_text_is_rendered_with_libass() {
        let mut f = Fixture::new(bluray_rip(), ClientFamily::Chrome);
        f.request.subtitle = Some(TrackRef::Embedded(3));
        let graph = value(&args_of(&f, 0.0, &[]), "-filter_complex");
        assert_eq!(
            graph,
            "[0:0]scale=w=1280:h=720:flags=bicubic,format=yuv420p[base];[0:3]scale=w=1280:h=720[sub];\
             [base][sub]overlay=eof_action=pass,format=yuv420p[vout]"
        );
        f.request.subtitle = Some(TrackRef::Embedded(4));
        f.request.burn_subtitle = true;
        let graph = value(&args_of(&f, 0.0, &[]), "-filter_complex");
        assert_eq!(
            graph,
            "[0:0]scale=w=1280:h=720:flags=bicubic,format=yuv420p,\
             subtitles=filename=/media/Film\\\\: Część 1.mkv:si=1,format=yuv420p[vout]"
        );
    }
}
