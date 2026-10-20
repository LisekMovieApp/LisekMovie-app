//! End-to-end checks of the HLS engine against the real FFmpeg on PATH.
//! Skipped (with a note) when `ffmpeg`/`ffprobe` are missing.
//!
//! Media is generated per test in a temporary directory: H.264 with
//! B-frames and 2 s GOPs, AAC 5.1, embedded SRT, an external CP1250 SRT,
//! and a 10-bit PQ-tagged HEVC clip for tone mapping.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use lisek_core::media_probe::MediaProbe;
use lisek_core::playback_settings::PlaybackSettings;
use lisek_playback::decision::{self, DecisionInput, PlayMethod, PlaybackRequest, VideoAction};
use lisek_playback::downmix::DownmixAlgorithm;
use lisek_playback::hls::Rendition;
use lisek_playback::keyframes::{self, SegmentMap};
use lisek_playback::profile::{ClientCapabilities, ClientFamily};
use lisek_playback::session::{self, SessionSpec};
use lisek_playback::subtitles::{self, TrackRef};
use lisek_playback::transcode;

fn have_ffmpeg() -> bool {
    let ok = |bin: &str| Command::new(bin).arg("-version").output().is_ok_and(|o| o.status.success());
    let found = ok("ffmpeg") && ok("ffprobe");
    if !found {
        eprintln!("ffmpeg/ffprobe not found: skipping");
    }
    found
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lisek-hls-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).output().unwrap();
    assert!(out.status.success(), "ffmpeg {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

const SRT: &str = "1\n00:00:01,000 --> 00:00:03,000\nPierwszy napis\n\n2\n00:00:21,500 --> 00:00:23,000\nZażółć gęślą jaźń\n\n3\n00:01:02,000 --> 00:01:04,000\nPo przewinięciu\n";

/// 90 s of 640x360 H.264 (B-frames, keyframe every 2 s) + AAC 5.1 + SRT.
fn make_movie(dir: &Path, name: &str) -> PathBuf {
    let srt = dir.join("embedded.srt");
    std::fs::write(&srt, SRT).unwrap();
    let path = dir.join(name);
    ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24000/1001:duration=90",
        "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000:duration=90",
        "-i", srt.to_str().unwrap(),
        "-filter_complex", "[1:a]pan=5.1|FL=c0|FR=c0|FC=c0|LFE=c0|BL=c0|BR=c0[a]",
        "-map", "0:v", "-map", "[a]", "-map", "2:s",
        "-c:v", "libx264", "-preset", "ultrafast", "-bf", "3", "-g", "48", "-keyint_min", "48",
        "-c:a", "aac", "-b:a", "256k", "-c:s", "srt", "-metadata:s:s:0", "language=pol",
        path.to_str().unwrap(),
    ]);
    path
}

fn probe(path: &Path) -> MediaProbe {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_streams", "-show_format", "-of", "json"])
        .arg(path)
        .output()
        .unwrap();
    transcode::parse_probe(&out.stdout).unwrap()
}

fn chrome() -> ClientCapabilities {
    ClientCapabilities::preset(ClientFamily::Chrome)
}

fn settings() -> PlaybackSettings {
    PlaybackSettings {
        encoder_preset: "ultrafast".into(),
        throttle_lead_seconds: 10,
        segment_retention_seconds: 10,
        ..Default::default()
    }
}

struct Started {
    session: std::sync::Arc<session::Session>,
    dir: PathBuf,
}

async fn start(
    name: &str,
    input: &Path,
    request: PlaybackRequest,
    settings: PlaybackSettings,
    position: f64,
    client: ClientCapabilities,
) -> Started {
    let probe = probe(input);
    let tools = transcode::tools(&settings).await;
    let keyframes = keyframes::matroska_keyframes(input).unwrap();
    let externals = subtitles::find_external(input);
    let plan = decision::decide(&DecisionInput {
        probe: &probe,
        externals: &externals,
        client: &client,
        settings: &settings,
        tools: &tools,
        request: &request,
        remote_limit_kbps: None,
        keyframes_available: keyframes.is_some(),
    })
    .unwrap();
    let origin = probe.start_time_seconds.unwrap_or(0.0);
    let duration = probe.duration_seconds.unwrap();
    let segments = match plan.video {
        VideoAction::Copy => SegmentMap::from_keyframes(&keyframes.unwrap(), origin, duration, 4.0).unwrap(),
        VideoAction::Encode(_) => SegmentMap::fixed(duration, settings.hls_segment_seconds as f64),
    };
    let text_subtitles: Vec<i32> = probe
        .streams
        .iter()
        .filter(|s| s.codec.as_deref().and_then(subtitles::kind_of_codec) == Some(subtitles::SubtitleKind::Text))
        .map(|s| s.index)
        .collect();
    let dir = scratch(name);
    let session = session::create(SessionSpec {
        id: format!("{name}-{}", std::process::id()),
        input: input.to_owned(),
        directory: dir.join("session"),
        plan,
        segments,
        origin,
        settings,
        tools,
        start_position: position,
        text_subtitles: text_subtitles.clone(),
        renditions: text_subtitles
            .iter()
            .map(|i| Rendition { track: TrackRef::Embedded(*i), name: "Polski".into(), language: Some("pol".into()), default: false, forced: false })
            .collect(),
        externals,
        frame_rate: Some(23.976),
    })
    .await
    .unwrap();
    Started { dir: dir.join("session"), session }
}

/// (first video pts, number of video packets) of init + one segment.
fn video_packets(init: &Path, segment: &Path, scratch: &Path) -> Vec<f64> {
    let joined = scratch.join("joined.mp4");
    let mut bytes = std::fs::read(init).unwrap();
    bytes.extend(std::fs::read(segment).unwrap());
    std::fs::write(&joined, bytes).unwrap();
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts_time", "-of", "csv=p=0"])
        .arg(&joined)
        .output()
        .unwrap();
    let mut pts: Vec<f64> = String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse().ok()).collect();
    pts.sort_by(f64::total_cmp);
    pts
}

fn segment_files(dir: &Path) -> BTreeSet<usize> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix("seg-")?.strip_suffix(".m4s")?.parse().ok())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remux_cuts_at_keyframes_throttles_seeks_and_exports_subtitles() {
    if !have_ffmpeg() {
        return;
    }
    let media = scratch("remux-media");
    let movie = make_movie(&media, "Film.mkv");

    // Cues give exactly the keyframes ffprobe finds by reading everything.
    let cues = keyframes::matroska_keyframes(&movie).unwrap().unwrap();
    let scanned = keyframes::scan_with_ffprobe(&movie, 0, &PlaybackSettings::default()).await.unwrap();
    assert_eq!(cues.len(), scanned.len());
    assert!(cues.iter().zip(&scanned).all(|(a, b)| (a - b).abs() < 0.002), "{cues:?} vs {scanned:?}");

    let started = start("remux", &movie, PlaybackRequest::default(), settings(), 0.0, chrome()).await;
    let s = &started.session;
    assert_eq!(s.spec().plan.method, PlayMethod::Remux);
    let map = s.spec().segments.clone();
    // Fragment timestamps are the file's own plus OUTPUT_TS_OFFSET;
    // playlist time zero is the file's start time.
    let zero = 10.0 + s.spec().origin;
    let init = s.init_segment().await.unwrap();
    let master = s.master_playlist("?ticket=t").await.unwrap();
    assert!(master.contains("CODECS=\"avc1.") && master.contains("mp4a.40.2"), "{master}");
    assert!(master.contains("SUBTITLES=\"subs\""));

    // Segment 0 starts at the first frame; segment 1 at its keyframe.
    for index in 0..3 {
        let path = s.media_segment(index).await.unwrap();
        let pts = video_packets(&init, &path, &started.dir.join(".."));
        let first = pts[0] - zero;
        assert!((first - map.start(index)).abs() < 0.03, "segment {index} starts at {first}, expected {}", map.start(index));
        assert!((pts[pts.len() - 1] - zero) < map.end(index));
    }

    // Throttling: with a 10 s lead FFmpeg stops a few segments ahead.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let produced = segment_files(&started.dir);
    let furthest = *produced.iter().max().unwrap();
    assert!(map.start(furthest) - map.start(2) <= 10.0 + map.longest(), "ran ahead to {furthest}: {produced:?}");
    assert!(furthest < map.len() - 5, "throttling never kicked in");

    // Subtitle cues arrive with the stream, cut per segment.
    let sub0 = s.subtitle_segment(TrackRef::Embedded(2), 0).await.unwrap();
    let mpegts = ((10.0 + s.spec().origin) * 90_000.0).round() as u64;
    assert!(sub0.contains(&format!("X-TIMESTAMP-MAP=MPEGTS:{mpegts},")) && sub0.contains("Pierwszy napis"), "{sub0}");

    // Seek to 60 s: FFmpeg restarts there; the old segments age out.
    let target = map.segment_at(60.0);
    let path = s.media_segment(target).await.unwrap();
    let pts = video_packets(&init, &path, &started.dir.join(".."));
    assert!((pts[0] - zero - map.start(target)).abs() < 0.03);
    let after_seek = s.subtitle_segment(TrackRef::Embedded(2), map.segment_at(62.5)).await.unwrap();
    assert!(after_seek.contains("Po przewinięciu"), "{after_seek}");
    // A subtitle segment before the run's start that no run produced is
    // answered empty after a short grace, not after hls.js gives up.
    let asked = std::time::Instant::now();
    let skipped = s.subtitle_segment(TrackRef::Embedded(2), target - 3).await.unwrap();
    assert!(asked.elapsed() < Duration::from_secs(4), "waited {:?}", asked.elapsed());
    assert!(!skipped.contains("-->"), "{skipped}");
    let produced = segment_files(&started.dir);
    assert!(!produced.contains(&0), "segments behind the retention window are deleted: {produced:?}");

    // Last segment and end of stream.
    let last = s.media_segment(map.len() - 1).await.unwrap();
    assert!(std::fs::metadata(last).unwrap().len() > 0);
    session::stop(&s.spec().id).await;
    assert!(!started.dir.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transcode_burns_external_cp1250_subtitles_and_downmixes() {
    if !have_ffmpeg() {
        return;
    }
    let media = scratch("burn-media");
    // A path FFmpeg's filter parser must have escaped.
    let movie = make_movie(&media, "Film: 'test' [x], y;.mkv");
    let (cp1250, _, _) = encoding_cp1250(SRT);
    std::fs::write(media.join("Film: 'test' [x], y;.pol.srt"), cp1250).unwrap();

    let externals = subtitles::find_external(&movie);
    assert_eq!(externals.len(), 1);
    let vtt = session::convert_external(&externals[0], &PlaybackSettings::default(), None).await.unwrap();
    assert!(vtt.contains("Zażółć gęślą jaźń"), "CP1250 decoded: {vtt}");

    for (track, name) in [(TrackRef::External(0), "burn-external"), (TrackRef::Embedded(2), "burn-embedded")] {
        let request = PlaybackRequest {
            subtitle: Some(track),
            burn_subtitle: true,
            downmix: lisek_playback::downmix::DownmixPreference::Night,
            ..Default::default()
        };
        let mut settings = settings();
        settings.max_output_height = 360;
        let started = start(name, &movie, request, settings, 20.0, chrome()).await;
        let s = &started.session;
        assert_eq!(s.spec().plan.method, PlayMethod::Transcode);
        let map = &s.spec().segments;
        let index = map.segment_at(21.0);
        assert_eq!(map.start(index), 20.0, "4 s grid");
        let init = s.init_segment().await.unwrap();
        let path = s.media_segment(index).await.unwrap();
        let zero = 10.0 + s.spec().origin;
        let pts = video_packets(&init, &path, &started.dir.join(".."));
        assert!((pts[0] - zero - 20.0).abs() < 0.05, "forced keyframe on the grid: {}", pts[0]);
        // The next segment also starts on the grid.
        let next = s.media_segment(index + 1).await.unwrap();
        let pts = video_packets(&init, &next, &started.dir.join(".."));
        assert!((pts[0] - zero - 24.0).abs() < 0.05, "{}", pts[0]);
        session::stop(&s.spec().id).await;
    }
}

/// UTF-8 to Windows-1250 for the Polish letters used in the samples.
fn encoding_cp1250(text: &str) -> (Vec<u8>, usize, bool) {
    let table = [('ą', 0xB9), ('ć', 0xE6), ('ę', 0xEA), ('ł', 0xB3), ('ń', 0xF1), ('ó', 0xF3), ('ś', 0x9C), ('ź', 0x9F), ('ż', 0xBF), ('Ż', 0xAF)];
    let bytes = text
        .chars()
        .map(|c| table.iter().find(|(k, _)| *k == c).map_or(c as u8, |(_, b)| *b))
        .collect::<Vec<u8>>();
    let len = bytes.len();
    (bytes, len, true)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hdr_is_tone_mapped_by_software_and_libplacebo() {
    if !have_ffmpeg() {
        return;
    }
    let media = scratch("hdr-media");
    let movie = media.join("hdr.mkv");
    ffmpeg(&[
        "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=24:duration=12",
        "-vf", "format=yuv420p10le",
        "-c:v", "libx265", "-preset", "ultrafast", "-x265-params", "log-level=error:keyint=48",
        "-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc",
        movie.to_str().unwrap(),
    ]);
    let mut client = chrome();
    client.video_codecs.push("hevc_10bit".into());
    let tools = transcode::tools(&PlaybackSettings::default()).await;
    let mut backends = vec!["software"];
    let vulkan = Command::new("ffmpeg")
        .args(["-v", "error", "-init_hw_device", "vulkan=vk:0", "-f", "lavfi", "-i", "nullsrc=s=16x16", "-frames:v", "1", "-f", "null", "-"])
        .output()
        .is_ok_and(|o| o.status.success());
    if vulkan && tools.has_filter("libplacebo") {
        backends.push("libplacebo");
    } else {
        eprintln!("no Vulkan device: libplacebo path not exercised");
    }
    for backend in backends {
        let mut settings = settings();
        settings.tonemapping = true;
        settings.tonemapping_backend = backend.into();
        let started = start(&format!("hdr-{backend}"), &movie, PlaybackRequest::default(), settings, 0.0, client.clone()).await;
        let s = &started.session;
        let VideoAction::Encode(encode) = &s.spec().plan.video else { panic!("{backend}: not transcoded") };
        assert_eq!(encode.tonemap.as_ref().unwrap().backend.as_str(), backend);
        let init = s.init_segment().await.unwrap();
        let segment = s.media_segment(0).await.unwrap();
        let joined = started.dir.join("check.mp4");
        let mut bytes = std::fs::read(&init).unwrap();
        bytes.extend(std::fs::read(segment).unwrap());
        std::fs::write(&joined, bytes).unwrap();
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=pix_fmt,color_transfer,color_primaries,height", "-of", "csv=p=0"])
            .arg(&joined)
            .output()
            .unwrap();
        let info = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        assert_eq!(info, "360,yuv420p,bt709,bt709", "{backend}");
        session::stop(&s.spec().id).await;
    }
}

#[test]
fn every_downmix_chain_runs_on_this_ffmpeg() {
    if !have_ffmpeg() {
        return;
    }
    for algorithm in [DownmixAlgorithm::Standard, DownmixAlgorithm::Dialogue, DownmixAlgorithm::Night, DownmixAlgorithm::ProLogicIi] {
        let out = Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i", "sine=f=300:d=1:sample_rate=48000"])
            .args(["-af", &format!("pan=7.1|c0=c0|c1=c0|c2=c0|c3=c0|c4=c0|c5=c0|c6=c0|c7=c0,{}", algorithm.filter(2.0))])
            .args(["-ac", "2", "-f", "null", "-"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{algorithm:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
}
