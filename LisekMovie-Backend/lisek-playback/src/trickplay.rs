//! Trickplay: the preview thumbnails a player shows above its seek bar.
//!
//! One FFmpeg pass per file decodes only keyframes (`-skip_frame nokey`,
//! tens of times faster than decoding every frame; short videos and files
//! with too few keyframes are decoded in full), keeps one frame every
//! [`Plan::interval`] seconds, scales it to a small tile and packs tiles
//! into JPEG sprite sheets (`tile=10x10`). Thumbnail `n` shows the moment
//! `n * interval`; it sits on sheet `n / 100` at column `n % 10`, row
//! `(n % 100) / 10`. HDR sources are tone mapped in software at tile size
//! (cheap at 320 px) so previews are not washed out.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use lisek_core::playback_settings::PlaybackSettings;
use tokio::io::AsyncReadExt;

use crate::transcode::{FfmpegTools, command};

/// Tile width in pixels; the height follows the picture's aspect ratio.
pub const TILE_WIDTH: u32 = 320;
pub const COLUMNS: u32 = 10;
pub const ROWS: u32 = 10;
/// Shortest and longest distance between thumbnails, in seconds.
pub const MIN_INTERVAL: u32 = 2;
pub const MAX_INTERVAL: u32 = 10;
/// Thumbnails per film to aim for before the interval reaches its limits.
const TARGET_COUNT: f64 = 200.0;

/// What one generation produces; stored with the item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    /// Seconds between thumbnails (whole seconds, so FFmpeg's `fps` is exact).
    pub interval: u32,
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    /// Thumbnails expected for the duration (the last sheet may be partial).
    pub count: u32,
}

impl Plan {
    /// `None` without a usable duration or picture size.
    pub fn new(duration_seconds: f64, source_width: u32, source_height: u32) -> Option<Plan> {
        if !duration_seconds.is_finite()
            || duration_seconds < 1.0
            || source_width < 2
            || source_height < 2
        {
            return None;
        }
        let interval =
            ((duration_seconds / TARGET_COUNT).round() as u32).clamp(MIN_INTERVAL, MAX_INTERVAL);
        let width = TILE_WIDTH.min(source_width & !1);
        let height = ((f64::from(width) * f64::from(source_height) / f64::from(source_width) / 2.0)
            .round() as u32
            * 2)
        .max(2);
        let count = (duration_seconds / f64::from(interval)).ceil().max(1.0) as u32;
        Some(Plan {
            interval,
            width,
            height,
            columns: COLUMNS,
            rows: ROWS,
            count,
        })
    }

    pub fn per_sheet(&self) -> u32 {
        self.columns * self.rows
    }

    pub fn sheets(&self) -> u32 {
        self.count.div_ceil(self.per_sheet())
    }
}

/// File name of sheet `index` inside an item's trickplay directory.
pub fn sheet_name(index: u32) -> String {
    format!("sheet-{index:03}.jpg")
}

/// HDR transfer characteristics that need tone mapping for SDR previews.
pub fn is_hdr_transfer(transfer: Option<&str>) -> bool {
    matches!(transfer, Some("smpte2084" | "arib-std-b67"))
}

/// The `-vf` chain: one frame per interval, scaled (and tone mapped for
/// HDR when the build can), packed into sheets.
pub fn filter_chain(plan: &Plan, hdr_transfer: Option<&str>, tools: &FfmpegTools) -> String {
    let mut filters = vec![
        format!("fps=1/{}", plan.interval),
        format!("scale={}:{}:flags=bilinear", plan.width, plan.height),
        "setsar=1".to_owned(),
    ];
    if let Some(transfer) = hdr_transfer.filter(|t| is_hdr_transfer(Some(t)))
        && tools.filters.contains("zscale")
        && tools.filters.contains("tonemap")
    {
        filters.push(format!(
            "setparams=color_primaries=bt2020:color_trc={transfer}:colorspace=bt2020nc"
        ));
        filters.push(format!(
            "zscale=tin={transfer}:min=bt2020nc:pin=bt2020:rin=tv:t=linear:npl=100"
        ));
        filters.push("format=gbrpf32le".to_owned());
        filters.push("zscale=p=bt709".to_owned());
        filters.push("tonemap=tonemap=hable:desat=0".to_owned());
        filters.push("zscale=t=bt709:m=bt709:r=tv".to_owned());
        filters.push("format=yuv420p".to_owned());
    }
    filters.push(format!("tile={}x{}", plan.columns, plan.rows));
    filters.join(",")
}

/// Below this length every frame is decoded: it costs seconds, and short
/// clips often have a single keyframe.
pub const KEYFRAMES_ONLY_FROM_SECONDS: u32 = 600;

/// FFmpeg arguments writing `sheet-000.jpg`, `sheet-001.jpg`, ... into
/// `out_dir`. `keyframes_only` decodes keyframes alone (`-skip_frame nokey`).
pub fn ffmpeg_args(
    input: &Path,
    out_dir: &Path,
    filter: &str,
    keyframes_only: bool,
) -> Vec<String> {
    let pattern = out_dir.join("sheet-%03d.jpg");
    let skip: &[&str] = if keyframes_only {
        &["-skip_frame", "nokey"]
    } else {
        &[]
    };
    [
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        // A background job: leave most cores to playback.
        "-threads",
        "2",
    ]
    .iter()
    .chain(skip)
    .chain(&["-i"])
    .map(|s| s.to_string())
    .chain([input.to_string_lossy().into_owned()])
    .chain(
        [
            "-map",
            "0:v:0",
            "-an",
            "-sn",
            "-dn",
            "-filter_threads",
            "1",
            "-vf",
            filter,
            "-q:v",
            "5",
            "-f",
            "image2",
            "-start_number",
            "0",
        ]
        .iter()
        .map(|s| s.to_string()),
    )
    .chain([pattern.to_string_lossy().into_owned()])
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrickplayError {
    /// FFmpeg could not be started (missing or wrong path).
    Spawn(String),
    /// FFmpeg failed; the last lines of its error output.
    Failed(String),
    Timeout,
    /// FFmpeg exited cleanly without writing a sheet.
    NoOutput,
    Io(String),
}

impl std::fmt::Display for TrickplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrickplayError::Spawn(e) => write!(f, "cannot start FFmpeg: {e}"),
            TrickplayError::Failed(e) => write!(f, "FFmpeg failed: {e}"),
            TrickplayError::Timeout => write!(f, "FFmpeg took too long"),
            TrickplayError::NoOutput => write!(f, "FFmpeg wrote no thumbnails"),
            TrickplayError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Longest a generation may run: a 3 h 4K film decodes its keyframes in a
/// few minutes; anything far beyond that is stuck.
fn time_limit(plan: &Plan) -> Duration {
    let seconds = u64::from(plan.count) * u64::from(plan.interval);
    Duration::from_secs((seconds / 4).clamp(300, 3600))
}

/// Generates the sheets for `input` into `out_dir` (created if missing).
/// Returns how many sheets were written. The caller swaps the directory
/// into place, so a failed run never replaces working thumbnails.
pub async fn generate(
    settings: &PlaybackSettings,
    tools: &FfmpegTools,
    input: &Path,
    out_dir: &Path,
    plan: &Plan,
    hdr_transfer: Option<&str>,
) -> Result<u32, TrickplayError> {
    tokio::fs::create_dir_all(out_dir)
        .await
        .map_err(|e| TrickplayError::Io(format!("cannot create {}: {e}", out_dir.display())))?;
    let filter = filter_chain(plan, hdr_transfer, tools);
    let long = plan.count * plan.interval >= KEYFRAMES_ONLY_FROM_SECONDS;
    match run(settings, input, out_dir, &filter, plan, long).await {
        // Too few keyframes for even one sheet: decode everything.
        Err(TrickplayError::NoOutput) if long => {
            run(settings, input, out_dir, &filter, plan, false).await
        }
        other => other,
    }
}

async fn run(
    settings: &PlaybackSettings,
    input: &Path,
    out_dir: &Path,
    filter: &str,
    plan: &Plan,
    keyframes_only: bool,
) -> Result<u32, TrickplayError> {
    let mut child = command("ffmpeg", settings)
        .args(ffmpeg_args(input, out_dir, filter, keyframes_only))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| TrickplayError::Spawn(e.to_string()))?;
    let mut stderr = child.stderr.take();
    let read_errors = async move {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.read_to_string(&mut text).await;
        }
        text
    };
    let run = async {
        let (status, errors) = tokio::join!(child.wait(), read_errors);
        (status, errors)
    };
    let (status, errors) = tokio::time::timeout(time_limit(plan), run)
        .await
        .map_err(|_| TrickplayError::Timeout)?;
    let status = status.map_err(|e| TrickplayError::Failed(e.to_string()))?;
    if !status.success() {
        let tail: Vec<&str> = errors.lines().rev().take(4).collect();
        return Err(TrickplayError::Failed(
            tail.into_iter().rev().collect::<Vec<_>>().join(" | "),
        ));
    }
    let written = count_sheets(out_dir).await;
    if written == 0 {
        return Err(TrickplayError::NoOutput);
    }
    Ok(written)
}

/// Consecutive sheets present from `sheet-000.jpg` on.
pub async fn count_sheets(dir: &Path) -> u32 {
    let mut n = 0;
    while tokio::fs::try_exists(dir.join(sheet_name(n)))
        .await
        .unwrap_or(false)
    {
        n += 1;
    }
    n
}

/// Where an item's sheets live under the server's configuration directory.
pub fn item_dir(root: &Path, item_id: &str) -> PathBuf {
    root.join(item_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plans_interval_tile_size_and_sheets() {
        // A two-hour 16:9 film: one thumbnail every 10 s, 8 sheets.
        let film = Plan::new(7200.0, 3840, 2160).unwrap();
        assert_eq!(
            (
                film.interval,
                film.width,
                film.height,
                film.count,
                film.sheets()
            ),
            (10, 320, 180, 720, 8)
        );
        // A 20-minute episode: every 6 s.
        let episode = Plan::new(1200.0, 1920, 800).unwrap();
        assert_eq!(
            (episode.interval, episode.height, episode.count),
            (6, 134, 200)
        );
        // Short clips keep the 2 s minimum; tiny sources are not upscaled.
        let clip = Plan::new(30.0, 240, 136).unwrap();
        assert_eq!(
            (
                clip.interval,
                clip.width,
                clip.height,
                clip.count,
                clip.sheets()
            ),
            (2, 240, 136, 15, 1)
        );
        assert!(Plan::new(f64::NAN, 1920, 1080).is_none());
        assert!(Plan::new(100.0, 0, 1080).is_none());
    }

    #[test]
    fn filter_tone_maps_hdr_only_when_the_build_can() {
        let plan = Plan::new(600.0, 3840, 2160).unwrap();
        let mut tools = FfmpegTools::default();
        let plain = filter_chain(&plan, Some("smpte2084"), &tools);
        assert_eq!(
            plain,
            "fps=1/3,scale=320:180:flags=bilinear,setsar=1,tile=10x10"
        );
        tools.filters.insert("zscale".into());
        tools.filters.insert("tonemap".into());
        let hdr = filter_chain(&plan, Some("smpte2084"), &tools);
        assert!(
            hdr.contains("tonemap=tonemap=hable") && hdr.ends_with("tile=10x10"),
            "{hdr}"
        );
        assert!(!filter_chain(&plan, Some("bt709"), &tools).contains("tonemap"));
        let args = ffmpeg_args(Path::new("/m/a.mkv"), Path::new("/t"), "x", true);
        let i = args.iter().position(|a| a == "-i").unwrap();
        assert!(
            args[..i].contains(&"nokey".to_string()),
            "keyframe-only decoding is an input option"
        );
        assert!(
            !ffmpeg_args(Path::new("/m/a.mkv"), Path::new("/t"), "x", false)
                .contains(&"nokey".to_string())
        );
        assert_eq!(
            args.last().unwrap(),
            &Path::new("/t").join("sheet-%03d.jpg").to_string_lossy()
        );
    }
}
