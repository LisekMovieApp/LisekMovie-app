//! Hardware diagnostics (Faza 13): what this machine can transcode with.
//!
//! A quick, read-only survey run when the administrator asks for it:
//! - the CPU model, cores and memory;
//! - GPUs: NVIDIA cards from `nvidia-smi`; on Linux every DRM render node
//!   (`/dev/dri/renderD*`, Intel/AMD integrated or dedicated) with its
//!   VA-API decode/encode profiles from `vainfo`; on Windows the video
//!   controllers WMI lists; Vulkan devices from `vulkaninfo`;
//! - the FFmpeg build: version, hardware decoders (`-hwaccels`) and
//!   hardware encoders;
//! - one tiny test encode per hardware encoder and device, because an
//!   encoder compiled into FFmpeg is not one the driver can run.
//!
//! Every external tool is optional: a missing one leaves its part empty.
//! Tools run with short time limits, tests in parallel, so the whole
//! survey takes a few seconds. Linux comes first; Windows and macOS get
//! the parts that exist there (WMI, `sysctl`, NVENC/QSV/AMF/VideoToolbox).
//!
//! Transcoding itself uses NVENC/NVDEC (and the CPU). Other working
//! encoders (VA-API, Quick Sync, AMF) are reported with `used_by_lisek:
//! false` so the administrator sees what the machine could do.

use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use lisek_core::playback_settings::{HardwareDevice, PlaybackSettings};
use serde::Serialize;
use tokio::process::Command;
use tokio::sync::Semaphore;

use crate::gpu::{self, NvidiaGpu};
use crate::transcode;

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct CpuInfo {
    pub model: Option<String>,
    pub logical_cores: usize,
    pub physical_cores: Option<usize>,
    pub memory_total_mb: Option<u64>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct GpuDevice {
    /// `nvidia`, `intel`, `amd` or `other`.
    pub vendor: String,
    pub name: String,
    /// `discrete`, `integrated` or `unknown`.
    pub kind: String,
    /// `nvidia-smi` index (NVIDIA only): the number transcodes use.
    pub index: Option<u32>,
    /// Linux render node, e.g. `/dev/dri/renderD128`.
    pub render_node: Option<String>,
    pub driver: Option<String>,
    pub memory_total_mb: Option<u64>,
    pub memory_used_mb: Option<u64>,
    pub utilization_percent: Option<u32>,
    /// LisekMovie transcodes on this GPU now.
    pub active_transcodes: usize,
    /// NVENC sessions of every process on this card.
    pub encoder_sessions: Option<u32>,
    /// Where this entry came from: `nvidia-smi`, `drm`, `wmi`.
    pub source: String,
}

/// VA-API capabilities of one render node (`vainfo`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, utoipa::ToSchema)]
pub struct VaapiDevice {
    pub render_node: String,
    pub driver: Option<String>,
    pub decode: Vec<String>,
    pub encode: Vec<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct EncoderTest {
    /// FFmpeg encoder, e.g. `hevc_nvenc`.
    pub encoder: String,
    /// `h264`, `hevc`, `av1`.
    pub codec: String,
    /// `nvenc`, `vaapi`, `qsv`, `amf`, `videotoolbox`.
    pub api: String,
    /// GPU index or render node the test ran on.
    pub device: Option<String>,
    pub ok: bool,
    /// Last line FFmpeg printed when the test failed.
    pub error: Option<String>,
    /// LisekMovie's transcoding uses this encoder.
    pub used_by_lisek: bool,
}

#[derive(Debug, Clone, Default, Serialize, utoipa::ToSchema)]
pub struct FfmpegInfo {
    pub path: String,
    pub version: Option<String>,
    /// `ffmpeg -hwaccels`: hardware decoding methods in this build.
    pub hwaccels: Vec<String>,
    /// Hardware encoders compiled into this build.
    pub hardware_encoders: Vec<String>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct DeviceLoad {
    /// `gpu::Target::id`, e.g. `nvenc:0`, `qsv:/dev/dri/renderD128`.
    pub id: String,
    pub api: String,
    pub name: String,
    pub weight: u32,
    pub max_sessions: u32,
    /// LisekMovie sessions on it now (starting ones included).
    pub transcodes: usize,
    /// In the configured set (a device just removed from the pool may
    /// still finish sessions).
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct BalancingInfo {
    /// The `hardware_acceleration` setting: `none`, `nvenc`, `qsv`, `vaapi` or `pool`.
    pub mode: String,
    /// Devices new transcodes are spread over, with their load.
    pub devices: Vec<DeviceLoad>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct HardwareReport {
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub duration_ms: u64,
    /// `linux`, `windows`, `macos` ...
    pub os: String,
    pub os_version: Option<String>,
    pub arch: String,
    /// Running in a container (Docker, Podman).
    pub container: bool,
    pub cpu: CpuInfo,
    pub gpus: Vec<GpuDevice>,
    pub vaapi: Vec<VaapiDevice>,
    pub vulkan_devices: Vec<String>,
    pub ffmpeg: FfmpegInfo,
    pub encoders: Vec<EncoderTest>,
    pub balancing: BalancingInfo,
    /// Devices the transcoding pool could use (Faza 14), from this survey:
    /// NVIDIA cards (NVENC), Intel GPUs (Quick Sync, else VA-API), AMD GPUs
    /// on Linux (VA-API); codecs from the tests and `vainfo`. Suggestions:
    /// `enabled` is off, the admin panel merges them with the saved pool.
    pub pool_candidates: Vec<HardwareDevice>,
    /// Things worth the administrator's attention, as codes:
    /// `no_hardware_encoder`, `nvenc_failed`, `acceleration_off`,
    /// `nvidia_smi_missing`, `other_encoders_unused`, `ffmpeg_missing`.
    pub warnings: Vec<String>,
}

// --- running tools ---------------------------------------------------------------

struct Output {
    success: bool,
    stdout: String,
    stderr: String,
}

async fn run(
    program: impl AsRef<std::ffi::OsStr>,
    args: &[&str],
    limit: Duration,
) -> Option<Output> {
    let mut command = Command::new(program);
    command
        .args(args)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(limit, command.output())
        .await
        .ok()?
        .ok()?;
    Some(Output {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

async fn run_ffmpeg(settings: &PlaybackSettings, args: &[&str], limit: Duration) -> Option<Output> {
    let mut command = transcode::command("ffmpeg", settings);
    command.args(args).stdin(std::process::Stdio::null());
    let output = tokio::time::timeout(limit, command.output())
        .await
        .ok()?
        .ok()?;
    // Diagnostics expect failures (a missing GPU): DEBUG, not WARN.
    crate::tool_log::debug(crate::tool_log::Tool::Ffmpeg, "hardware diagnostics", &output.stderr);
    Some(Output {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

// --- parsers (pure, tested) -------------------------------------------------------

/// `model name`, and physical cores from distinct (`physical id`, `core id`).
pub fn parse_cpuinfo(text: &str) -> (Option<String>, Option<usize>) {
    let mut model = None;
    let mut cores = std::collections::HashSet::new();
    let mut physical = String::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "model name" | "Model" | "Hardware" if model.is_none() && !value.is_empty() => {
                model = Some(value.to_owned())
            }
            "physical id" => physical = value.to_owned(),
            "core id" => {
                cores.insert((physical.clone(), value.to_owned()));
            }
            _ => {}
        }
    }
    (model, (!cores.is_empty()).then_some(cores.len()))
}

/// `MemTotal` in MiB.
pub fn parse_meminfo(text: &str) -> Option<u64> {
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

fn vaapi_codec(profile: &str) -> Option<&'static str> {
    let p = profile.trim_start_matches("VAProfile");
    Some(match () {
        _ if p.starts_with("H264") => "h264",
        _ if p.starts_with("HEVC") => "hevc",
        _ if p.starts_with("AV1") => "av1",
        _ if p.starts_with("VP9") => "vp9",
        _ if p.starts_with("VP8") => "vp8",
        _ if p.starts_with("MPEG2") => "mpeg2",
        _ if p.starts_with("VC1") => "vc1",
        _ if p.starts_with("JPEG") => "mjpeg",
        _ => return None,
    })
}

/// `vainfo` output: driver line and profile/entrypoint pairs.
pub fn parse_vainfo(render_node: &str, text: &str) -> VaapiDevice {
    let mut device = VaapiDevice {
        render_node: render_node.to_owned(),
        ..Default::default()
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.split_once("Driver version:").map(|(_, r)| r.trim()) {
            device.driver = Some(rest.to_owned());
            continue;
        }
        let Some((profile, entry)) = line.split_once(':') else {
            continue;
        };
        let Some(codec) = vaapi_codec(profile.trim()) else {
            continue;
        };
        let entry = entry.trim();
        let list = if entry == "VAEntrypointVLD" {
            &mut device.decode
        } else if entry.starts_with("VAEntrypointEncSlice") {
            &mut device.encode
        } else {
            continue;
        };
        if !list.iter().any(|c| c == codec) {
            list.push(codec.to_owned());
        }
    }
    device
}

/// `deviceName` entries of `vulkaninfo --summary`, software renderers left out.
pub fn parse_vulkaninfo(text: &str) -> Vec<String> {
    let mut names: Vec<String> = text
        .lines()
        .filter_map(|l| {
            l.trim()
                .strip_prefix("deviceName")
                .map(|r| r.trim_start_matches([' ', '=']).trim().to_owned())
        })
        .filter(|n| !n.is_empty() && !n.to_ascii_lowercase().contains("llvmpipe"))
        .collect();
    names.dedup();
    names
}

/// Hardware encoders in `ffmpeg -encoders` output.
pub fn parse_hw_encoders(text: &str) -> Vec<String> {
    const SUFFIXES: [&str; 5] = ["_nvenc", "_vaapi", "_qsv", "_amf", "_videotoolbox"];
    text.lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .filter(|name| {
            SUFFIXES.iter().any(|s| name.ends_with(s))
                && ["h264", "hevc", "av1"].iter().any(|c| name.starts_with(c))
        })
        .map(str::to_owned)
        .collect()
}

/// `ffmpeg -hwaccels` list.
pub fn parse_hwaccels(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| !l.starts_with("Hardware acceleration methods"))
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

fn vendor_name(id: &str) -> &'static str {
    match id.trim().to_ascii_lowercase().as_str() {
        "0x10de" => "nvidia",
        "0x8086" => "intel",
        "0x1002" | "0x1022" => "amd",
        _ => "other",
    }
}

fn vendor_from_name(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("quadro") || n.contains("tesla")
    {
        "nvidia"
    } else if n.contains("intel") {
        "intel"
    } else if n.contains("amd") || n.contains("radeon") {
        "amd"
    } else {
        "other"
    }
}

/// The line of FFmpeg's error output that says why: the first one that
/// reads like a cause ("Cannot load libcuda.so.1"), not the generic
/// trailer ("Nothing was written into output file"), else the last line.
fn last_error(stderr: &str) -> Option<String> {
    const GENERIC: [&str; 3] = [
        "nothing was written",
        "conversion failed",
        "error while filtering",
    ];
    const CAUSE: [&str; 7] = [
        "cannot",
        "could not",
        "failed",
        "error",
        "not found",
        "unsupported",
        "no device",
    ];
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let cause = lines.iter().find(|l| {
        let lower = l.to_ascii_lowercase();
        CAUSE.iter().any(|c| lower.contains(c)) && !GENERIC.iter().any(|g| lower.contains(g))
    });
    cause
        .or(lines.last())
        .map(|l| l.chars().take(200).collect())
}

// --- survey -----------------------------------------------------------------------

async fn cpu() -> CpuInfo {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let mut info = CpuInfo {
        logical_cores: logical,
        ..Default::default()
    };
    if cfg!(target_os = "linux") {
        if let Ok(text) = tokio::fs::read_to_string("/proc/cpuinfo").await {
            (info.model, info.physical_cores) = parse_cpuinfo(&text);
        }
        if let Ok(text) = tokio::fs::read_to_string("/proc/meminfo").await {
            info.memory_total_mb = parse_meminfo(&text);
        }
    } else if cfg!(windows) {
        let script = "$c=Get-CimInstance Win32_Processor|Select-Object -First 1;$m=(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory;\"$($c.Name)`n$($c.NumberOfCores)`n$m\"";
        if let Some(out) = run(
            "powershell",
            &["-NoProfile", "-NonInteractive", "-Command", script],
            Duration::from_secs(8),
        )
        .await
        {
            let mut lines = out.stdout.lines().map(str::trim);
            info.model = lines.next().filter(|l| !l.is_empty()).map(str::to_owned);
            info.physical_cores = lines.next().and_then(|l| l.parse().ok());
            info.memory_total_mb = lines
                .next()
                .and_then(|l| l.parse::<u64>().ok())
                .map(|b| b / 1024 / 1024);
        }
    } else if cfg!(target_os = "macos")
        && let Some(out) = run(
            "sysctl",
            &[
                "-n",
                "machdep.cpu.brand_string",
                "hw.physicalcpu",
                "hw.memsize",
            ],
            Duration::from_secs(5),
        )
        .await
    {
        let mut lines = out.stdout.lines().map(str::trim);
        info.model = lines.next().filter(|l| !l.is_empty()).map(str::to_owned);
        info.physical_cores = lines.next().and_then(|l| l.parse().ok());
        info.memory_total_mb = lines
            .next()
            .and_then(|l| l.parse::<u64>().ok())
            .map(|b| b / 1024 / 1024);
    }
    info
}

async fn read_trimmed(path: PathBuf) -> Option<String> {
    tokio::fs::read_to_string(path)
        .await
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Linux render nodes: vendor, driver, PCI slot, VRAM (AMD).
async fn drm_devices() -> Vec<(String, GpuDevice, Option<String>)> {
    let mut out = Vec::new();
    let Ok(mut dir) = tokio::fs::read_dir("/sys/class/drm").await else {
        return out;
    };
    let mut nodes = Vec::new();
    while let Ok(Some(entry)) = dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("renderD") {
            nodes.push(name);
        }
    }
    nodes.sort();
    for node in nodes {
        let device = PathBuf::from("/sys/class/drm").join(&node).join("device");
        let vendor = read_trimmed(device.join("vendor"))
            .await
            .map_or("other", |v| vendor_name(&v))
            .to_owned();
        let driver = tokio::fs::read_link(device.join("driver"))
            .await
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        let slot = read_trimmed(device.join("uevent")).await.and_then(|u| {
            u.lines()
                .find_map(|l| l.strip_prefix("PCI_SLOT_NAME=").map(str::to_owned))
        });
        let vram = read_trimmed(device.join("mem_info_vram_total"))
            .await
            .and_then(|v| v.parse::<u64>().ok())
            .map(|b| b / 1024 / 1024);
        let kind = match (vendor.as_str(), vram) {
            ("intel", _) => "integrated",
            ("nvidia", _) => "discrete",
            ("amd", Some(mb)) if mb <= 2048 => "integrated",
            ("amd", Some(_)) => "discrete",
            _ => "unknown",
        };
        out.push((
            node.clone(),
            GpuDevice {
                vendor: vendor.clone(),
                name: String::new(),
                kind: kind.into(),
                index: None,
                render_node: Some(format!("/dev/dri/{node}")),
                driver,
                memory_total_mb: vram,
                memory_used_mb: None,
                utilization_percent: None,
                active_transcodes: 0,
                encoder_sessions: None,
                source: "drm".into(),
            },
            slot,
        ));
    }
    out
}

async fn pci_name(slot: &str) -> Option<String> {
    let out = run("lspci", &["-mm", "-s", slot], Duration::from_secs(3)).await?;
    // 00:02.0 "VGA compatible controller" "Intel Corporation" "Alder Lake-S GT1 [UHD Graphics 770]" ...
    let quoted: Vec<&str> = out.stdout.split('"').skip(1).step_by(2).collect();
    match (quoted.get(1), quoted.get(2)) {
        (Some(vendor), Some(device)) => Some(format!("{vendor} {device}")),
        _ => None,
    }
}

async fn windows_gpus() -> Vec<GpuDevice> {
    let script = "Get-CimInstance Win32_VideoController|ForEach-Object{\"$($_.Name)|$($_.AdapterRAM)|$($_.DriverVersion)\"}";
    let Some(out) = run(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", script],
        Duration::from_secs(8),
    )
    .await
    else {
        return Vec::new();
    };
    out.stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.trim().split('|');
            let name = parts.next()?.trim().to_owned();
            if name.is_empty() {
                return None;
            }
            let memory = parts
                .next()
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(|b| b / 1024 / 1024);
            let driver = parts
                .next()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty());
            let vendor = vendor_from_name(&name);
            let kind = match vendor {
                "intel" => "integrated",
                "nvidia" => "discrete",
                _ => "unknown",
            };
            Some(GpuDevice {
                vendor: vendor.into(),
                name,
                kind: kind.into(),
                index: None,
                render_node: None,
                driver,
                memory_total_mb: memory,
                memory_used_mb: None,
                utilization_percent: None,
                active_transcodes: 0,
                encoder_sessions: None,
                source: "wmi".into(),
            })
        })
        .collect()
}

struct TestPlan {
    encoder: String,
    codec: String,
    api: String,
    device: Option<String>,
    args: Vec<String>,
}

/// One test encode per hardware encoder and device: NVENC per NVIDIA GPU,
/// VA-API per Intel/AMD render node, Quick Sync per Intel render node
/// (Linux) or on the first Intel adapter (Windows, `auto`). Devices are
/// opened exactly as transcoding opens them (`command::intel_device_args`).
fn test_plans(
    encoders: &[String],
    nvidia: &[NvidiaGpu],
    render_nodes: &[(String, String)],
) -> Vec<TestPlan> {
    let source = [
        "-hide_banner",
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=25",
        "-frames:v",
        "5",
    ];
    let build = |before: Vec<std::ffi::OsString>, after: &[&str], encoder: &str| -> Vec<String> {
        let mut args: Vec<String> = before
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        args.extend(source.iter().map(|s| (*s).to_owned()));
        args.extend(after.iter().map(|s| (*s).to_owned()));
        args.extend(["-c:v", encoder, "-f", "null", "-"].map(str::to_owned));
        args
    };
    let mut plans = Vec::new();
    for encoder in encoders {
        let codec = encoder.split('_').next().unwrap_or_default().to_owned();
        let api = encoder.rsplit('_').next().unwrap_or_default().to_owned();
        let mut push = |device: Option<String>, args: Vec<String>| {
            plans.push(TestPlan {
                encoder: encoder.clone(),
                codec: codec.clone(),
                api: api.clone(),
                device,
                args,
            });
        };
        match api.as_str() {
            "nvenc" => {
                let indices: Vec<u32> = if nvidia.is_empty() {
                    vec![0]
                } else {
                    nvidia.iter().map(|g| g.index).collect()
                };
                for index in indices {
                    let mut args = build(Vec::new(), &[], encoder);
                    let at = args.len() - 3;
                    args.splice(at..at, ["-gpu".to_owned(), index.to_string()]);
                    push(Some(index.to_string()), args);
                }
            }
            "vaapi" => {
                for (node, vendor) in render_nodes.iter().filter(|(_, v)| v != "nvidia") {
                    let _ = vendor;
                    let before = crate::command::intel_device_args("vaapi", node);
                    push(
                        Some(node.clone()),
                        build(before, &["-vf", "format=nv12,hwupload"], encoder),
                    );
                }
            }
            "qsv" => {
                let intel: Vec<&String> = render_nodes
                    .iter()
                    .filter(|(_, v)| v == "intel")
                    .map(|(n, _)| n)
                    .collect();
                let devices: Vec<String> = if intel.is_empty() {
                    if render_nodes.is_empty() {
                        vec!["auto".to_owned()]
                    } else {
                        Vec::new()
                    }
                } else {
                    intel.into_iter().cloned().collect()
                };
                for device in devices {
                    let before = crate::command::intel_device_args("qsv", &device);
                    push(
                        Some(device),
                        build(
                            before,
                            &["-vf", "format=nv12,hwupload=extra_hw_frames=64,format=qsv"],
                            encoder,
                        ),
                    );
                }
            }
            _ => push(None, build(Vec::new(), &[], encoder)),
        }
    }
    plans
}

/// FFmpeg decoder names for `vainfo` codec names.
fn decode_name(codec: &str) -> &str {
    if codec == "mpeg2" {
        "mpeg2video"
    } else {
        codec
    }
}

/// Pool suggestions from a survey (see [`HardwareReport::pool_candidates`]).
fn pool_candidates(
    nvidia: &[NvidiaGpu],
    drm: &[(String, String, String)],
    vaapi: &[VaapiDevice],
    wmi_intel: Option<&str>,
    encoders: &[EncoderTest],
    settings: &PlaybackSettings,
) -> Vec<HardwareDevice> {
    let working = |api: &str, device: &str| -> Vec<String> {
        let mut codecs: Vec<String> = encoders
            .iter()
            .filter(|e| {
                e.ok && e.api == api
                    && e.device.as_deref() == Some(device)
                    && matches!(e.codec.as_str(), "h264" | "hevc")
            })
            .map(|e| e.codec.clone())
            .collect();
        codecs.sort();
        codecs.dedup();
        codecs
    };
    let mut out = Vec::new();
    for g in nvidia {
        let device = g.index.to_string();
        out.push(HardwareDevice {
            api: "nvenc".into(),
            encode_codecs: working("nvenc", &device),
            device,
            name: g.name.clone(),
            enabled: false,
            weight: 2,
            max_sessions: 0,
            decode_codecs: settings.hardware_decoding_codecs.clone(),
        });
    }
    for (node, vendor, name) in drm {
        if vendor == "nvidia" || !matches!(vendor.as_str(), "intel" | "amd") {
            continue;
        }
        // Without `vainfo` (not in the Docker image) the common codecs of
        // current Intel/AMD GPUs are suggested; the admin can change them.
        let decode: Vec<String> = match vaapi.iter().find(|v| &v.render_node == node) {
            Some(v) => v
                .decode
                .iter()
                .map(|c| decode_name(c).to_owned())
                .filter(|c| c != "mjpeg")
                .collect(),
            None => ["h264", "hevc", "vp9", "av1"].map(String::from).to_vec(),
        };
        let qsv = working("qsv", node);
        let (api, encode) = if vendor == "intel" && !qsv.is_empty() {
            ("qsv", qsv)
        } else {
            ("vaapi", working("vaapi", node))
        };
        // Quick Sync has no MPEG-4 Part 2 decoder.
        let decode = if api == "qsv" {
            decode.into_iter().filter(|c| c != "mpeg4").collect()
        } else {
            decode
        };
        out.push(HardwareDevice {
            api: api.into(),
            device: node.clone(),
            name: name.clone(),
            enabled: false,
            weight: 1,
            max_sessions: 0,
            decode_codecs: decode,
            encode_codecs: encode,
        });
    }
    if let Some(name) = wmi_intel {
        out.push(HardwareDevice {
            api: "qsv".into(),
            device: "auto".into(),
            name: name.to_owned(),
            enabled: false,
            weight: 1,
            max_sessions: 0,
            decode_codecs: ["h264", "hevc", "vp9", "av1"].map(String::from).to_vec(),
            encode_codecs: working("qsv", "auto"),
        });
    }
    out
}

static LAST: LazyLock<Mutex<Option<HardwareReport>>> = LazyLock::new(|| Mutex::new(None));

/// The latest report, if diagnostics ran since the server started.
pub fn last_report() -> Option<HardwareReport> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Live balancing state: the configured devices and their sessions.
pub async fn balancing(settings: &PlaybackSettings) -> BalancingInfo {
    let (targets, _) = gpu::configured_targets(settings).await;
    let load = gpu::load();
    let mut devices: Vec<DeviceLoad> = targets
        .iter()
        .map(|t| DeviceLoad {
            id: t.id.clone(),
            api: t.api.clone(),
            name: t.name.clone(),
            weight: t.weight,
            max_sessions: t.max_sessions,
            transcodes: load.get(&t.id).copied().unwrap_or(0),
            configured: true,
        })
        .collect();
    let mut others: Vec<(&String, &usize)> = load
        .iter()
        .filter(|(id, _)| !targets.iter().any(|t| &t.id == *id))
        .collect();
    others.sort();
    for (id, transcodes) in others {
        devices.push(DeviceLoad {
            id: id.clone(),
            api: id.split(':').next().unwrap_or_default().to_owned(),
            name: String::new(),
            weight: 0,
            max_sessions: 0,
            transcodes: *transcodes,
            configured: false,
        });
    }
    BalancingInfo {
        mode: settings.hardware_acceleration.clone(),
        devices,
    }
}

/// Runs the survey and keeps the result for [`last_report`].
pub async fn diagnose(settings: &PlaybackSettings) -> HardwareReport {
    let started = std::time::Instant::now();
    let ffmpeg_path = transcode::binary_path("ffmpeg", settings)
        .display()
        .to_string();
    let linux = cfg!(target_os = "linux");

    let os_version = async {
        if linux {
            read_trimmed("/proc/sys/kernel/osrelease".into()).await
        } else if cfg!(windows) {
            run("cmd", &["/C", "ver"], Duration::from_secs(5))
                .await
                .map(|o| o.stdout.trim().to_owned())
                .filter(|s| !s.is_empty())
        } else {
            run("uname", &["-r"], Duration::from_secs(5))
                .await
                .map(|o| o.stdout.trim().to_owned())
        }
    };
    let (os_version, cpu, nvidia, drm, wmi, vulkan, version, hwaccels, encoders) = tokio::join!(
        os_version,
        cpu(),
        gpu::refresh_nvidia(),
        async {
            if linux {
                drm_devices().await
            } else {
                Vec::new()
            }
        },
        async {
            if cfg!(windows) {
                windows_gpus().await
            } else {
                Vec::new()
            }
        },
        run("vulkaninfo", &["--summary"], Duration::from_secs(8)),
        run_ffmpeg(
            settings,
            &["-hide_banner", "-version"],
            Duration::from_secs(10)
        ),
        run_ffmpeg(
            settings,
            &["-hide_banner", "-hwaccels"],
            Duration::from_secs(10)
        ),
        run_ffmpeg(
            settings,
            &["-hide_banner", "-encoders"],
            Duration::from_secs(10)
        ),
    );
    let container = linux
        && (tokio::fs::try_exists("/.dockerenv").await.unwrap_or(false)
            || tokio::fs::try_exists("/run/.containerenv")
                .await
                .unwrap_or(false));

    // GPUs: nvidia-smi first; render nodes and WMI entries of the same
    // NVIDIA cards are left out.
    let active = gpu::load();
    let mut gpus: Vec<GpuDevice> = nvidia
        .iter()
        .map(|g| GpuDevice {
            vendor: "nvidia".into(),
            name: g.name.clone(),
            kind: "discrete".into(),
            index: Some(g.index),
            render_node: None,
            driver: g.driver_version.clone(),
            memory_total_mb: g.memory_total_mb,
            memory_used_mb: g.memory_used_mb,
            utilization_percent: g.utilization_percent,
            active_transcodes: active
                .get(&format!("nvenc:{}", g.index))
                .copied()
                .unwrap_or(0),
            encoder_sessions: g.encoder_sessions,
            source: "nvidia-smi".into(),
        })
        .collect();
    let mut render_nodes: Vec<(String, String)> = Vec::new();
    let mut drm_named: Vec<(String, String, String)> = Vec::new();
    let mut vaapi_tasks = Vec::new();
    for (node, mut device, slot) in drm {
        let path = format!("/dev/dri/{node}");
        render_nodes.push((path.clone(), device.vendor.clone()));
        if device.vendor == "nvidia" && !nvidia.is_empty() {
            continue;
        }
        if let Some(slot) = &slot {
            device.name = pci_name(slot).await.unwrap_or_default();
        }
        if device.name.is_empty() {
            device.name = format!("{} GPU ({node})", device.vendor);
        }
        device.active_transcodes = ["qsv", "vaapi"]
            .iter()
            .map(|api| active.get(&format!("{api}:{path}")).copied().unwrap_or(0))
            .sum();
        drm_named.push((path.clone(), device.vendor.clone(), device.name.clone()));
        gpus.push(device);
        vaapi_tasks.push(async move {
            run(
                "vainfo",
                &["--display", "drm", "--device", &path],
                Duration::from_secs(8),
            )
            .await
            .filter(|o| o.success)
            .map(|o| parse_vainfo(&path, &o.stdout))
        });
    }
    let vaapi: Vec<VaapiDevice> = futures_join_all(vaapi_tasks)
        .await
        .into_iter()
        .flatten()
        .collect();
    let mut wmi_intel = None;
    for mut device in wmi {
        if device.vendor == "nvidia" && !nvidia.is_empty() {
            continue;
        }
        if device.vendor == "intel" && wmi_intel.is_none() {
            wmi_intel = Some(device.name.clone());
            device.active_transcodes = active.get("qsv:auto").copied().unwrap_or(0);
        }
        gpus.push(device);
    }

    let ffmpeg = FfmpegInfo {
        path: ffmpeg_path,
        version: version
            .as_ref()
            .filter(|o| o.success)
            .and_then(|o| o.stdout.lines().next().map(str::to_owned)),
        hwaccels: hwaccels
            .as_ref()
            .map(|o| parse_hwaccels(&o.stdout))
            .unwrap_or_default(),
        hardware_encoders: encoders
            .as_ref()
            .map(|o| parse_hw_encoders(&o.stdout))
            .unwrap_or_default(),
    };

    // Test encodes, four at a time. VA-API only where render nodes exist.
    let plans = test_plans(&ffmpeg.hardware_encoders, &nvidia, &render_nodes);
    let limit = Arc::new(Semaphore::new(4));
    let tests = plans.into_iter().map(|plan| {
        let limit = limit.clone();
        async move {
            let _permit = limit.acquire().await.ok();
            let args: Vec<&str> = plan.args.iter().map(String::as_str).collect();
            let output = run_ffmpeg(settings, &args, Duration::from_secs(15)).await;
            let (ok, error) = match output {
                Some(o) if o.success => (true, None),
                Some(o) => (
                    false,
                    last_error(&o.stderr).or_else(|| Some("ffmpeg_failed".into())),
                ),
                None => (false, Some("timed_out".into())),
            };
            EncoderTest {
                used_by_lisek: matches!(plan.api.as_str(), "nvenc" | "qsv" | "vaapi")
                    && plan.codec != "av1",
                encoder: plan.encoder,
                codec: plan.codec,
                api: plan.api,
                device: plan.device,
                ok,
                error,
            }
        }
    });
    let encoders: Vec<EncoderTest> = futures_join_all(tests).await;

    let mut warnings = Vec::new();
    if ffmpeg.version.is_none() {
        warnings.push("ffmpeg_missing".to_owned());
    }
    if !encoders.iter().any(|e| e.ok) {
        warnings.push("no_hardware_encoder".to_owned());
    }
    // NVENC failing on a machine without an NVIDIA card is expected.
    let has_nvidia = !nvidia.is_empty() || gpus.iter().any(|g| g.vendor == "nvidia");
    if has_nvidia && encoders.iter().any(|e| e.api == "nvenc" && !e.ok) {
        warnings.push("nvenc_failed".to_owned());
    }
    if encoders.iter().any(|e| e.ok && e.used_by_lisek) && settings.hardware_acceleration == "none"
    {
        warnings.push("acceleration_off".to_owned());
    }
    if nvidia.is_empty() && encoders.iter().any(|e| e.api == "nvenc" && e.ok) {
        warnings.push("nvidia_smi_missing".to_owned());
    }
    if encoders
        .iter()
        .any(|e| e.ok && !e.used_by_lisek && matches!(e.api.as_str(), "amf" | "videotoolbox"))
    {
        warnings.push("other_encoders_unused".to_owned());
    }
    let pooled = |api: &str, device: &str| {
        settings.hardware_acceleration == "pool"
            && settings
                .hardware_pool
                .iter()
                .any(|d| d.enabled && d.api == api && d.device == device)
    };
    if encoders.iter().any(|e| {
        e.ok && matches!(e.api.as_str(), "qsv" | "vaapi")
            && !pooled(&e.api, e.device.as_deref().unwrap_or_default())
    }) && !matches!(settings.hardware_acceleration.as_str(), "qsv" | "vaapi")
    {
        warnings.push("igpu_not_in_pool".to_owned());
    }
    let pool_candidates = pool_candidates(
        &nvidia,
        &drm_named,
        &vaapi,
        wmi_intel.as_deref(),
        &encoders,
        settings,
    );

    let report = HardwareReport {
        generated_at: chrono::Utc::now(),
        duration_ms: started.elapsed().as_millis() as u64,
        os: std::env::consts::OS.into(),
        os_version,
        arch: std::env::consts::ARCH.into(),
        container,
        cpu,
        gpus,
        vaapi,
        vulkan_devices: vulkan
            .filter(|o| o.success)
            .map(|o| parse_vulkaninfo(&o.stdout))
            .unwrap_or_default(),
        ffmpeg,
        encoders,
        balancing: balancing(settings).await,
        pool_candidates,
        warnings,
    };
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(report.clone());
    report
}

/// Runs futures concurrently, results in order (no extra dependency).
async fn futures_join_all<F: std::future::Future>(
    futures: impl IntoIterator<Item = F>,
) -> Vec<F::Output> {
    use std::task::Poll;
    let mut pending: Vec<Option<std::pin::Pin<Box<F>>>> =
        futures.into_iter().map(|f| Some(Box::pin(f))).collect();
    let mut outputs: Vec<Option<F::Output>> = (0..pending.len()).map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut done = true;
        for (slot, output) in pending.iter_mut().zip(outputs.iter_mut()) {
            if let Some(future) = slot {
                match future.as_mut().poll(cx) {
                    Poll::Ready(value) => {
                        *output = Some(value);
                        *slot = None;
                    }
                    Poll::Pending => done = false,
                }
            }
        }
        if done { Poll::Ready(()) } else { Poll::Pending }
    })
    .await;
    outputs.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_and_memory() {
        let cpuinfo = "processor\t: 0\nmodel name\t: Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz\nphysical id\t: 0\ncore id\t\t: 0\n\n\
                       processor\t: 1\nmodel name\t: Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz\nphysical id\t: 0\ncore id\t\t: 0\n\n\
                       processor\t: 2\nphysical id\t: 1\ncore id\t\t: 0\n";
        assert_eq!(
            parse_cpuinfo(cpuinfo),
            (
                Some("Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz".into()),
                Some(2)
            )
        );
        assert_eq!(
            parse_cpuinfo("Hardware\t: BCM2835\n"),
            (Some("BCM2835".into()), None)
        );
        assert_eq!(
            parse_meminfo("MemTotal:       65748612 kB\nMemFree: 1 kB\n"),
            Some(64207)
        );
    }

    #[test]
    fn vainfo_profiles() {
        let text = "Trying display: drm\nvainfo: VA-API version: 1.20 (libva 2.20.0)\n\
                    vainfo: Driver version: Intel iHD driver for Intel(R) Gen Graphics - 24.1.0\n\
                    vainfo: Supported profile and entrypoints\n\
                          VAProfileH264Main               :\tVAEntrypointVLD\n\
                          VAProfileH264Main               :\tVAEntrypointEncSliceLP\n\
                          VAProfileH264High               :\tVAEntrypointEncSlice\n\
                          VAProfileHEVCMain10             :\tVAEntrypointVLD\n\
                          VAProfileAV1Profile0            :\tVAEntrypointVLD\n\
                          VAProfileJPEGBaseline           :\tVAEntrypointEncPicture\n\
                          VAProfileNone                   :\tVAEntrypointVideoProc\n";
        let d = parse_vainfo("/dev/dri/renderD128", text);
        assert_eq!(
            d.driver.as_deref(),
            Some("Intel iHD driver for Intel(R) Gen Graphics - 24.1.0")
        );
        assert_eq!(d.decode, ["h264", "hevc", "av1"]);
        assert_eq!(d.encode, ["h264"]);
    }

    #[test]
    fn ffmpeg_lists() {
        let encoders = " V....D libx264              libx264 H.264\n V....D h264_nvenc           NVIDIA NVENC H.264 encoder\n \
                        V....D hevc_vaapi           H.265/HEVC (VAAPI)\n V....D av1_qsv  AV1 (Intel Quick Sync Video)\n V....D mjpeg_vaapi x\n";
        assert_eq!(
            parse_hw_encoders(encoders),
            ["h264_nvenc", "hevc_vaapi", "av1_qsv"]
        );
        assert_eq!(
            parse_hwaccels("Hardware acceleration methods:\ncuda\nvaapi\n\n"),
            ["cuda", "vaapi"]
        );
        assert_eq!(
            parse_vulkaninfo(
                "GPU0:\n\tdeviceName         = NVIDIA GeForce RTX 3060\nGPU1:\n\tdeviceName = llvmpipe (LLVM 17)\n"
            ),
            ["NVIDIA GeForce RTX 3060"]
        );
        assert_eq!(vendor_name("0x8086"), "intel");
        assert_eq!(vendor_from_name("AMD Radeon(TM) Graphics"), "amd");
        assert_eq!(
            last_error("x\nError while opening encoder\n\n").as_deref(),
            Some("Error while opening encoder")
        );
        assert_eq!(
            last_error("[h264_nvenc @ 0x1] Cannot load libcuda.so.1\n[out#0/null @ 0x2] Nothing was written into output file\n").as_deref(),
            Some("[h264_nvenc @ 0x1] Cannot load libcuda.so.1")
        );
        assert_eq!(last_error("plain\nlast\n").as_deref(), Some("last"));
    }

    #[test]
    fn pool_suggestions_follow_the_tests() {
        let nvidia = crate::gpu::parse_nvidia_smi("0, NVIDIA RTX 3060, x, 1, 12288, 1, 1, 0\n");
        let drm = vec![
            (
                "/dev/dri/renderD128".to_owned(),
                "intel".to_owned(),
                "Intel UHD 770".to_owned(),
            ),
            (
                "/dev/dri/renderD129".to_owned(),
                "amd".to_owned(),
                "AMD Radeon".to_owned(),
            ),
        ];
        let vaapi = vec![VaapiDevice {
            render_node: "/dev/dri/renderD128".into(),
            driver: None,
            decode: vec![
                "h264".into(),
                "hevc".into(),
                "mpeg2".into(),
                "vp8".into(),
                "mjpeg".into(),
            ],
            encode: vec!["h264".into()],
        }];
        let test = |encoder: &str, device: &str, ok: bool| EncoderTest {
            encoder: encoder.into(),
            codec: encoder.split('_').next().unwrap().into(),
            api: encoder.rsplit('_').next().unwrap().into(),
            device: Some(device.into()),
            ok,
            error: None,
            used_by_lisek: true,
        };
        let tests = vec![
            test("h264_nvenc", "0", true),
            test("hevc_nvenc", "0", true),
            test("h264_qsv", "/dev/dri/renderD128", true),
            test("hevc_qsv", "/dev/dri/renderD128", false),
            test("h264_vaapi", "/dev/dri/renderD129", true),
        ];
        let list = pool_candidates(
            &nvidia,
            &drm,
            &vaapi,
            None,
            &tests,
            &PlaybackSettings::default(),
        );
        let ids: Vec<String> = list.iter().map(HardwareDevice::id).collect();
        assert_eq!(
            ids,
            [
                "nvenc:0",
                "qsv:/dev/dri/renderD128",
                "vaapi:/dev/dri/renderD129"
            ]
        );
        assert_eq!(list[0].encode_codecs, ["h264", "hevc"]);
        assert_eq!(list[1].encode_codecs, ["h264"], "only what passed the test");
        assert_eq!(
            list[1].decode_codecs,
            ["h264", "hevc", "mpeg2video", "vp8"],
            "MJPEG left out"
        );
        assert!(list.iter().all(|d| !d.enabled && d.validate()));
    }

    #[test]
    fn one_test_per_encoder_and_device() {
        let nvidia =
            crate::gpu::parse_nvidia_smi("0, A, x, 1, 100, 1, 1, 0\n1, B, y, 1, 100, 1, 1, 0\n");
        let plans = test_plans(
            &["h264_nvenc".into(), "hevc_vaapi".into(), "h264_qsv".into()],
            &nvidia,
            &[
                ("/dev/dri/renderD128".into(), "intel".into()),
                ("/dev/dri/renderD129".into(), "amd".into()),
            ],
        );
        let summary: Vec<(String, Option<String>)> = plans
            .iter()
            .map(|p| (p.encoder.clone(), p.device.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                ("h264_nvenc".into(), Some("0".into())),
                ("h264_nvenc".into(), Some("1".into())),
                ("hevc_vaapi".into(), Some("/dev/dri/renderD128".into())),
                ("hevc_vaapi".into(), Some("/dev/dri/renderD129".into())),
                ("h264_qsv".into(), Some("/dev/dri/renderD128".into())),
            ]
        );
        let qsv = plans[4].args.join(" ");
        assert!(qsv.starts_with("-init_hw_device vaapi=va:/dev/dri/renderD128 -init_hw_device qsv=qs@va -filter_hw_device qs"), "{qsv}");
        assert!(
            qsv.contains("-vf format=nv12,hwupload=extra_hw_frames=64,format=qsv -c:v h264_qsv"),
            "{qsv}"
        );
        // Windows: no render nodes, Quick Sync on the first Intel adapter.
        let windows = test_plans(&["h264_qsv".into()], &[], &[]);
        assert_eq!(windows[0].device.as_deref(), Some("auto"));
        assert!(
            windows[0]
                .args
                .join(" ")
                .contains("qsv=qs:hw_any,child_device_type=d3d11va")
        );
        let nvenc = &plans[1].args;
        let gpu = nvenc.iter().position(|a| a == "-gpu").unwrap();
        assert_eq!(nvenc[gpu + 1], "1");
        assert_eq!(
            &nvenc[nvenc.len() - 7..],
            ["-c:v", "h264_nvenc", "-gpu", "1", "-f", "null", "-"]
        );
    }
}
