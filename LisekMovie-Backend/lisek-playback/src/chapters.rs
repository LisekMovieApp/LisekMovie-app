//! Chapter (scene) thumbnails: one JPEG per chapter, cut from the file on
//! first request and cached by the caller.
//!
//! The frame is taken a few seconds into the chapter (its first frames are
//! often a fade from black), sought before decoding (`-ss` ahead of `-i`,
//! keyframe-fast), scaled down first and tone mapped in software for HDR,
//! like trickplay sheets.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use lisek_core::playback_settings::PlaybackSettings;
use tokio::io::AsyncReadExt;

use crate::transcode::{FfmpegTools, command};
use crate::trickplay::{TrickplayError, tone_map_filters};

/// Thumbnail width in pixels (height keeps the aspect ratio): sharp on a
/// 240 px card at 2x density.
pub const THUMBNAIL_WIDTH: u32 = 480;
/// How far into a chapter the frame is taken, at most.
const OFFSET_SECONDS: f64 = 5.0;
/// A single sought frame takes well under a second; anything longer hangs.
const TIME_LIMIT: Duration = Duration::from_secs(30);

/// The moment shown for a chapter: a few seconds in, but never past its
/// first third (short chapters) nor its end.
pub fn thumbnail_time(start: f64, end: Option<f64>) -> f64 {
    let start = start.max(0.0);
    let offset = match end {
        Some(end) if end > start => OFFSET_SECONDS.min((end - start) / 3.0),
        Some(_) => 0.0,
        None => OFFSET_SECONDS,
    };
    start + offset
}

/// `-vf`: scale, square pixels, tone map HDR.
pub fn filter(hdr_transfer: Option<&str>, tools: &FfmpegTools) -> String {
    let mut filters = vec![
        format!("scale={THUMBNAIL_WIDTH}:-2:flags=bicubic"),
        "setsar=1".to_owned(),
    ];
    filters.extend(tone_map_filters(hdr_transfer, tools));
    filters.join(",")
}

/// FFmpeg arguments writing one JPEG of stream `video_index` at `at`.
pub fn ffmpeg_args(
    input: &Path,
    video_index: i32,
    at: f64,
    filter: &str,
    output: &Path,
) -> Vec<String> {
    [
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-threads",
        "2",
        "-ss",
        &format!("{at:.3}"),
        "-i",
        &input.to_string_lossy(),
        "-map",
        &format!("0:{video_index}"),
        "-an",
        "-sn",
        "-dn",
        "-frames:v",
        "1",
        "-vf",
        filter,
        "-q:v",
        "4",
        "-f",
        "image2",
        "-y",
        &output.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Writes the thumbnail to `output` (through a temporary file next to it,
/// so a half-written image is never served).
pub async fn generate(
    settings: &PlaybackSettings,
    tools: &FfmpegTools,
    input: &Path,
    video_index: i32,
    at: f64,
    hdr_transfer: Option<&str>,
    output: &Path,
) -> Result<(), TrickplayError> {
    let dir = output
        .parent()
        .ok_or_else(|| TrickplayError::Io("no output folder".into()))?;
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| TrickplayError::Io(format!("cannot create {}: {e}", dir.display())))?;
    let temp = output.with_extension(format!("{}.tmp.jpg", std::process::id()));
    let mut child = command("ffmpeg", settings)
        .args(ffmpeg_args(
            input,
            video_index,
            at,
            &filter(hdr_transfer, tools),
            &temp,
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TrickplayError::Spawn(e.to_string()))?;
    let mut stderr = child.stderr.take();
    let run = async {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.read_to_string(&mut text).await;
        }
        (child.wait().await, text)
    };
    let outcome = tokio::time::timeout(TIME_LIMIT, run).await;
    let result = match outcome {
        Err(_) => Err(TrickplayError::Timeout),
        Ok((Err(e), _)) => Err(TrickplayError::Failed(e.to_string())),
        Ok((Ok(status), errors)) if !status.success() => Err(TrickplayError::Failed(
            errors.lines().rev().take(3).collect::<Vec<_>>().join(" | "),
        )),
        Ok(_) => match tokio::fs::metadata(&temp).await {
            Ok(meta) if meta.len() > 0 => tokio::fs::rename(&temp, output)
                .await
                .map_err(|e| TrickplayError::Io(e.to_string())),
            _ => Err(TrickplayError::NoOutput),
        },
    };
    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_is_taken_a_little_into_the_chapter() {
        assert_eq!(thumbnail_time(0.0, Some(300.0)), 5.0);
        assert_eq!(thumbnail_time(100.0, Some(106.0)), 102.0);
        assert_eq!(thumbnail_time(100.0, Some(100.0)), 100.0);
        assert_eq!(thumbnail_time(100.0, None), 105.0);
        assert_eq!(thumbnail_time(-3.0, None), 5.0);
    }

    #[test]
    fn the_command_seeks_before_decoding_one_frame_of_the_right_stream() {
        let tools = FfmpegTools::default();
        let args = ffmpeg_args(
            Path::new("/m/a.mkv"),
            1,
            65.5,
            &filter(None, &tools),
            Path::new("/c/1.jpg"),
        );
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        let input = args.iter().position(|a| a == "-i").unwrap();
        assert!(ss < input, "input seeking is keyframe-fast");
        assert_eq!(args[ss + 1], "65.500");
        assert!(args.windows(2).any(|w| w == ["-map", "0:1"]));
        assert!(args.windows(2).any(|w| w == ["-frames:v", "1"]));
        assert_eq!(filter(None, &tools), "scale=480:-2:flags=bicubic,setsar=1");
        let mut capable = FfmpegTools::default();
        capable.filters.insert("zscale".into());
        capable.filters.insert("tonemap".into());
        assert!(filter(Some("smpte2084"), &capable).contains("tonemap=tonemap=hable"));
        assert!(!filter(Some("bt709"), &capable).contains("tonemap"));
    }

    /// ffprobe reads a Matroska file's chapters (real FFmpeg; skipped
    /// without one).
    #[tokio::test]
    async fn ffprobe_reads_chapters_from_a_real_file() {
        let Ok(output) = std::process::Command::new("ffprobe").arg("-version").output() else {
            return;
        };
        if !output.status.success() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("lisek-chapters-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let meta = dir.join("chapters.txt");
        std::fs::write(
            &meta,
            ";FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=4000\ntitle=Chapter 1\n\
             [CHAPTER]\nTIMEBASE=1/1000\nSTART=4000\nEND=10000\ntitle=Po\u{15b}cig\n",
        )
        .unwrap();
        let input = dir.join("in.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=10", "-i"])
            .arg(&meta)
            .args(["-map_metadata", "1", "-map_chapters", "1", "-t", "10"])
            .arg(&input)
            .status()
            .unwrap();
        assert!(made.success());
        let probe = crate::transcode::probe_media(&input, &PlaybackSettings::default())
            .await
            .unwrap();
        let summary: Vec<_> = probe
            .chapters
            .iter()
            .map(|c| (c.start_seconds.round() as i64, c.title.as_deref()))
            .collect();
        assert_eq!(summary, [(0, None), (4, Some("Po\u{15b}cig"))]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Against the real FFmpeg when there is one (skipped otherwise).
    #[tokio::test]
    async fn generates_a_jpeg_with_the_installed_ffmpeg() {
        let Ok(output) = std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
        else {
            return;
        };
        if !output.status.success() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("lisek-chapter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=10",
                "-t",
                "12",
            ])
            .arg(&input)
            .status()
            .unwrap();
        assert!(made.success());
        let out = dir.join("thumbs").join("0.jpg");
        let settings = PlaybackSettings::default();
        generate(
            &settings,
            &FfmpegTools::default(),
            &input,
            0,
            6.0,
            None,
            &out,
        )
        .await
        .unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a JPEG");
        let leftovers = std::fs::read_dir(out.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
