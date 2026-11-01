//! Faza 21: whether a hardware transcoding device actually works here.
//!
//! A machine can list an encoder (`h264_vaapi`) and a render node
//! (`/dev/dri/renderD128`) and still be unable to open them: no VA-API
//! driver for that GPU (`intel-media-va-driver` / `i965-va-driver`), a
//! kernel without the device, a container without `/dev/dri`. FFmpeg then
//! stops at once with
//! `Failed to initialise VAAPI connection: -1` /
//! `Failed to set value 'vaapi=va:/dev/dri/renderD128' for option 'init_hw_device'`.
//!
//! Two layers keep playback going:
//! - before a session picks a device, [`check`] knows (or finds out with a
//!   one-frame test encode, a fraction of a second) whether it works; a
//!   device that does not is skipped and the session plans for the CPU;
//! - a run that still fails with a hardware error ([`is_hardware_failure`])
//!   marks its device broken and restarts on the CPU (`session`).
//!
//! Verdicts come from those tests, from FFmpeg runs and from the admin's
//! hardware diagnostics. A failure is retried after [`FAILED_TTL`] (a
//! driver may have been installed meanwhile), a success re-checked after
//! [`OK_TTL`].

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use lisek_core::playback_settings::PlaybackSettings;
use serde::Serialize;

use crate::decision::{OutputCodec, hardware_encoder};
use crate::gpu::Target;

pub const OK_TTL: Duration = Duration::from_secs(60 * 60);
pub const FAILED_TTL: Duration = Duration::from_secs(15 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Where a verdict came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The one-frame test before a session.
    Probe,
    /// Admin → Hardware diagnostics.
    Diagnostics,
    /// A transcode that failed with a hardware error.
    Runtime,
}

#[derive(Debug, Clone)]
struct Entry {
    ok: bool,
    error: Option<String>,
    source: Source,
    at: DateTime<Utc>,
    checked: Instant,
}

/// One device's verdict, for the admin panel.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DeviceHealth {
    /// `gpu::Target::id`, e.g. `vaapi:/dev/dri/renderD128`, `nvenc:0`.
    pub id: String,
    pub ok: bool,
    /// The FFmpeg line that explains a failure.
    pub error: Option<String>,
    pub source: Source,
    pub checked_at: DateTime<Utc>,
    /// Still in force (failures are retried after 15 minutes).
    pub current: bool,
}

static HEALTH: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);
/// One test at a time: sessions starting together wait for the first one.
static PROBING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

fn table() -> std::sync::MutexGuard<'static, HashMap<String, Entry>> {
    HEALTH.lock().unwrap_or_else(|e| e.into_inner())
}

fn fresh(entry: &Entry) -> bool {
    entry.checked.elapsed() < if entry.ok { OK_TTL } else { FAILED_TTL }
}

/// The verdict for `id` while it is still in force.
pub fn known(id: &str) -> Option<bool> {
    table().get(id).filter(|e| fresh(e)).map(|e| e.ok)
}

pub fn record(id: &str, ok: bool, error: Option<String>, source: Source) {
    let previous = table().insert(
        id.to_owned(),
        Entry { ok, error: error.clone(), source, at: Utc::now(), checked: Instant::now() },
    );
    let changed = previous.is_none_or(|p| p.ok != ok);
    if !ok && changed {
        tracing::warn!(device = id, ?source, error = error.as_deref().unwrap_or(""), "hardware transcoding unavailable on this device; using the CPU");
    } else if ok && changed {
        tracing::info!(device = id, ?source, "hardware transcoding works on this device");
    }
}

/// Forgets every verdict (settings changed, "test again").
pub fn reset() {
    table().clear();
}

pub fn snapshot() -> Vec<DeviceHealth> {
    let mut list: Vec<DeviceHealth> = table()
        .iter()
        .map(|(id, e)| DeviceHealth {
            id: id.clone(),
            ok: e.ok,
            error: e.error.clone(),
            source: e.source,
            checked_at: e.at,
            current: fresh(e),
        })
        .collect();
    list.sort_by(|a, b| a.id.cmp(&b.id));
    list
}

/// FFmpeg output that means the GPU (driver, device, runtime) failed, not
/// the file: such a run is worth repeating on the CPU.
pub fn is_hardware_failure(log: &str) -> bool {
    const SIGNS: &[&str] = &[
        "failed to initialise vaapi",
        "for option 'init_hw_device'",
        "device creation failed",
        "no va display found",
        "vainitialize failed",
        "libva error",
        "va_openDriver() returns",
        "hwaccel initialisation returned error",
        "failed setup for format vaapi",
        "failed setup for format qsv",
        "failed setup for format cuda",
        "error creating a mfx session",
        "error initializing an internal mfx session",
        "error initializing the mfx video core",
        "mfx_err",
        "cannot load libcuda",
        "cannot load nvcuda",
        "cuda_error",
        "cuinit",
        "no nvenc capable devices found",
        "openencodesessionex failed",
        "nvenc api version",
        "failed to create vulkan device",
        "failed to create device",
        "error creating a vaapi",
        "failed to upload frame",
        "hwupload",
    ];
    let lower = log.to_ascii_lowercase();
    SIGNS.iter().any(|sign| lower.contains(&sign.to_ascii_lowercase()))
}

/// The line of FFmpeg output worth showing.
pub fn explain(log: &str) -> Option<String> {
    log.lines()
        .rev()
        .map(str::trim)
        .find(|line| is_hardware_failure(line))
        .or_else(|| log.lines().rev().map(str::trim).find(|l| !l.is_empty()))
        .map(|line| line.chars().take(300).collect())
}

/// FFmpeg arguments that open `target` and encode one frame on it (or,
/// for a decode-only device, only open it).
pub fn probe_args(target: &Target) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-v", "error", "-nostdin"].map(String::from).to_vec();
    let encodes = target.encode_codecs.iter().any(|c| c == "h264");
    match target.api.as_str() {
        "nvenc" => {
            args.extend(["-init_hw_device".into(), format!("cuda=cu:{}", target.device)]);
        }
        api => {
            args.extend(
                crate::command::intel_device_args(api, &target.device)
                    .into_iter()
                    .map(|a| a.to_string_lossy().into_owned()),
            );
        }
    }
    args.extend(["-f", "lavfi", "-i", "color=c=black:s=256x144:r=25", "-frames:v", "1"].map(String::from));
    if encodes {
        match target.api.as_str() {
            "vaapi" => args.extend(["-vf".into(), "format=nv12,hwupload".into()]),
            "qsv" => args.extend(["-vf".into(), "format=nv12,hwupload=extra_hw_frames=16".into()]),
            _ => {}
        }
        args.extend(["-c:v".into(), hardware_encoder(&target.api, OutputCodec::H264)]);
        if target.api == "nvenc" {
            args.extend(["-gpu".into(), target.device.clone()]);
        }
    }
    args.extend(["-f", "null", "-"].map(String::from));
    args
}

/// Whether `target` works, testing it once when nothing is known yet.
pub async fn check(target: &Target, settings: &PlaybackSettings) -> bool {
    if let Some(ok) = known(&target.id) {
        return ok;
    }
    let _guard = PROBING.lock().await;
    // Another session may have tested it while this one waited.
    if let Some(ok) = known(&target.id) {
        return ok;
    }
    let mut command = crate::transcode::command("ffmpeg", settings);
    command.args(probe_args(target)).stdin(std::process::Stdio::null());
    let started = Instant::now();
    let (ok, error) = match tokio::time::timeout(PROBE_TIMEOUT, command.output()).await {
        Ok(Ok(output)) if output.status.success() => (true, None),
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            crate::tool_log::debug(crate::tool_log::Tool::Ffmpeg, "hardware check", &output.stderr);
            (false, explain(&stderr).or_else(|| Some("ffmpeg_failed".into())))
        }
        Ok(Err(_)) => (false, Some("ffmpeg_unavailable".into())),
        Err(_) => (false, Some("timed_out".into())),
    };
    tracing::debug!(device = %target.id, ok, ms = started.elapsed().as_millis() as u64, "hardware check");
    record(&target.id, ok, error, Source::Probe);
    ok
}

/// Records Admin → Hardware diagnostics' H.264 test encodes as verdicts
/// (`api:device`, the same ids the device pool uses).
pub fn record_diagnostics(tests: &[crate::hardware::EncoderTest]) {
    for test in tests {
        if test.codec != "h264" || !matches!(test.api.as_str(), "nvenc" | "qsv" | "vaapi") {
            continue;
        }
        let device = test.device.clone().unwrap_or_else(|| "auto".into());
        record(&format!("{}:{device}", test.api), test.ok, test.error.clone(), Source::Diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(api: &str, device: &str, encodes: bool) -> Target {
        Target {
            id: format!("{api}:{device}"),
            api: api.into(),
            device: device.into(),
            name: String::new(),
            weight: 1,
            max_sessions: 0,
            decode_codecs: vec!["h264".into()],
            encode_codecs: if encodes { vec!["h264".into()] } else { vec![] },
        }
    }

    #[test]
    fn recognises_the_vaapi_failure_from_the_report() {
        let log = "[AVHWDeviceContext @ 0x55] Failed to initialise VAAPI connection: -1 (unknown libva error).\n\
                   Failed to set value 'vaapi=va:/dev/dri/renderD128' for option 'init_hw_device': Input/output error\n\
                   Error parsing global options: Input/output error";
        assert!(is_hardware_failure(log));
        assert_eq!(
            explain(log).unwrap(),
            "Failed to set value 'vaapi=va:/dev/dri/renderD128' for option 'init_hw_device': Input/output error"
        );
        assert!(is_hardware_failure("Error creating a MFX session: -9."));
        assert!(is_hardware_failure("Cannot load libcuda.so.1"));
        assert!(!is_hardware_failure("moov atom not found\nInvalid data found when processing input"));
    }

    #[test]
    fn probes_open_the_device_and_encode_one_frame() {
        let vaapi = probe_args(&target("vaapi", "/dev/dri/renderD128", true)).join(" ");
        assert!(vaapi.contains("-init_hw_device vaapi=va:/dev/dri/renderD128"));
        assert!(vaapi.contains("hwupload") && vaapi.contains("-c:v h264_vaapi") && vaapi.contains("-frames:v 1"));
        let decode_only = probe_args(&target("qsv", "/dev/dri/renderD129", false)).join(" ");
        assert!(decode_only.contains("qsv=qs@va") && !decode_only.contains("-c:v"));
        let nvenc = probe_args(&target("nvenc", "1", true)).join(" ");
        assert!(nvenc.contains("-init_hw_device cuda=cu:1") && nvenc.contains("-c:v h264_nvenc -gpu 1"));
    }

    #[test]
    fn verdicts_expire_and_diagnostics_count() {
        record("vaapi:/test/a", false, Some("x".into()), Source::Runtime);
        assert_eq!(known("vaapi:/test/a"), Some(false));
        record("vaapi:/test/a", true, None, Source::Probe);
        assert_eq!(known("vaapi:/test/a"), Some(true));
        assert_eq!(known("vaapi:/test/unknown"), None);
        let tests = vec![crate::hardware::EncoderTest {
            encoder: "h264_vaapi".into(),
            codec: "h264".into(),
            api: "vaapi".into(),
            device: Some("/test/b".into()),
            ok: false,
            error: Some("Failed to initialise VAAPI connection: -1".into()),
            used_by_lisek: true,
        }];
        record_diagnostics(&tests);
        let entry = snapshot().into_iter().find(|d| d.id == "vaapi:/test/b").unwrap();
        assert!(!entry.ok && entry.current && entry.source == Source::Diagnostics);
    }
}
