//! Sharing transcodes between GPUs: several NVIDIA cards (Faza 13), and
//! since Faza 14 a pool mixing NVIDIA, Intel Quick Sync and VA-API
//! devices ([`Target`], [`pick`], [`reserve`]).
//!
//! Every session that decodes or encodes video on the GPU gets one device
//! when it starts ([`crate::session::create`]), and all its FFmpeg runs
//! use it (`-init_hw_device cuda=cu:N`, `-gpu N`). The device is the
//! candidate with the fewest GPU sessions right now; on a tie, the one
//! with the most free video memory (from `nvidia-smi`, sampled at most
//! every few seconds); then round robin. So two simultaneous transcodes
//! land on two cards, and a card whose memory is taken by something else
//! (a game, another service) gets new work last.
//!
//! Candidates: the administrator's list (`hardware_devices`), or every
//! GPU `nvidia-smi` reports; without `nvidia-smi` (or with balancing off)
//! only `hardware_device_index`. Device numbers follow `nvidia-smi`'s PCI
//! order, which FFmpeg also uses because LisekMovie starts it with
//! `CUDA_DEVICE_ORDER=PCI_BUS_ID` (see `transcode::command`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use lisek_core::playback_settings::PlaybackSettings;
use serde::Serialize;

/// One NVIDIA GPU as `nvidia-smi` reports it.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct NvidiaGpu {
    /// `nvidia-smi` index (PCI bus order).
    pub index: u32,
    pub name: String,
    pub pci_bus_id: Option<String>,
    pub driver_version: Option<String>,
    pub memory_total_mb: Option<u64>,
    pub memory_used_mb: Option<u64>,
    pub utilization_percent: Option<u32>,
    /// NVENC sessions open on the card (all processes).
    pub encoder_sessions: Option<u32>,
}

impl NvidiaGpu {
    pub fn memory_free_mb(&self) -> Option<u64> {
        Some(self.memory_total_mb?.saturating_sub(self.memory_used_mb?))
    }
}

/// Fields asked from `nvidia-smi --query-gpu`, in [`parse_nvidia_smi`]'s order.
pub const NVIDIA_QUERY: &str = "index,name,pci.bus_id,driver_version,memory.total,memory.used,utilization.gpu,encoder.stats.sessionCount";

/// Parses `nvidia-smi --query-gpu=<NVIDIA_QUERY> --format=csv,noheader,nounits`.
/// Values a card cannot report (`[N/A]`, `[Not Supported]`) become `None`.
pub fn parse_nvidia_smi(csv: &str) -> Vec<NvidiaGpu> {
    csv.lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split(',').map(str::trim).collect();
            if cols.len() < 2 {
                return None;
            }
            let text = |i: usize| {
                cols.get(i)
                    .filter(|v| !v.is_empty() && !v.starts_with('['))
                    .map(|v| (*v).to_owned())
            };
            let number = |i: usize| {
                text(i)
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| *v >= 0.0)
            };
            Some(NvidiaGpu {
                index: cols[0].parse().ok()?,
                name: text(1)?,
                pci_bus_id: text(2),
                driver_version: text(3),
                memory_total_mb: number(4).map(|v| v as u64),
                memory_used_mb: number(5).map(|v| v as u64),
                utilization_percent: number(6).map(|v| v as u32),
                encoder_sessions: number(7).map(|v| v as u32),
            })
        })
        .collect()
}

fn nvidia_smi_binary() -> std::ffi::OsString {
    std::env::var_os("LISEK_NVIDIA_SMI").unwrap_or_else(|| "nvidia-smi".into())
}

/// Asks `nvidia-smi` now. Empty when it is missing, fails or hangs.
pub async fn query_nvidia() -> Vec<NvidiaGpu> {
    let mut command = tokio::process::Command::new(nvidia_smi_binary());
    command
        .args([
            &format!("--query-gpu={NVIDIA_QUERY}"),
            "--format=csv,noheader,nounits",
        ])
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    match tokio::time::timeout(Duration::from_secs(3), command.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

struct Sample {
    at: Instant,
    gpus: Vec<NvidiaGpu>,
}
static SAMPLE: LazyLock<Mutex<Option<Sample>>> = LazyLock::new(|| Mutex::new(None));
/// How long a sample is reused when choosing a device.
const SAMPLE_TTL: Duration = Duration::from_secs(5);
/// How long "no nvidia-smi here" is remembered.
const MISSING_TTL: Duration = Duration::from_secs(60);

/// `nvidia-smi`'s view, reused for a few seconds.
pub async fn nvidia_gpus() -> Vec<NvidiaGpu> {
    if let Some(sample) = SAMPLE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let ttl = if sample.gpus.is_empty() {
            MISSING_TTL
        } else {
            SAMPLE_TTL
        };
        if sample.at.elapsed() < ttl {
            return sample.gpus.clone();
        }
    }
    refresh_nvidia().await
}

/// Samples `nvidia-smi` now and keeps the result (diagnostics).
pub async fn refresh_nvidia() -> Vec<NvidiaGpu> {
    let gpus = query_nvidia().await;
    *SAMPLE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sample {
        at: Instant::now(),
        gpus: gpus.clone(),
    });
    gpus
}

/// The devices transcodes may use, in preference order.
pub fn candidates(settings: &PlaybackSettings, detected: &[NvidiaGpu]) -> Vec<u32> {
    if !settings.gpu_load_balancing {
        return vec![settings.hardware_device_index];
    }
    let mut list: Vec<u32> = if !settings.hardware_devices.is_empty() {
        settings.hardware_devices.clone()
    } else {
        detected.iter().map(|g| g.index).collect()
    };
    let mut seen = std::collections::HashSet::new();
    list.retain(|d| seen.insert(*d));
    if list.is_empty() {
        list.push(settings.hardware_device_index);
    }
    list
}

/// Memory differences below this do not decide between two cards.
const MEMORY_STEP_MB: u64 = 512;

/// A device sessions can be given (Faza 14): an NVIDIA GPU (NVENC/NVDEC),
/// an Intel GPU (Quick Sync) or an Intel/AMD GPU through VA-API.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct Target {
    /// `api:device`, e.g. `nvenc:0`, `qsv:/dev/dri/renderD128`.
    pub id: String,
    /// `nvenc`, `qsv` or `vaapi`.
    pub api: String,
    pub device: String,
    pub name: String,
    /// Share of sessions (see [`pick`]).
    pub weight: u32,
    /// 0: no limit.
    pub max_sessions: u32,
    pub decode_codecs: Vec<String>,
    pub encode_codecs: Vec<String>,
}

/// Default device of a single-device `qsv` / `vaapi` setting.
fn default_intel_device(api: &str) -> &'static str {
    if cfg!(windows) && api == "qsv" {
        "auto"
    } else {
        "/dev/dri/renderD128"
    }
}

/// The devices the settings allow, in preference order:
/// - `nvenc`: the NVIDIA GPUs of Faza 13 ([`candidates`]), sharing the
///   global decode/encode settings;
/// - `qsv` / `vaapi`: the one device in `hardware_device_path`;
/// - `pool`: the administrator's enabled devices, each with its own
///   share, limit and codecs.
pub fn targets(settings: &PlaybackSettings, detected: &[NvidiaGpu]) -> Vec<Target> {
    let encode = if settings.hardware_encoding {
        settings.hardware_encode_codecs.clone()
    } else {
        Vec::new()
    };
    match settings.hardware_acceleration.as_str() {
        "nvenc" => candidates(settings, detected)
            .into_iter()
            .map(|index| Target {
                id: format!("nvenc:{index}"),
                api: "nvenc".into(),
                device: index.to_string(),
                name: detected
                    .iter()
                    .find(|g| g.index == index)
                    .map(|g| g.name.clone())
                    .unwrap_or_default(),
                weight: 1,
                max_sessions: 0,
                decode_codecs: settings.hardware_decoding_codecs.clone(),
                encode_codecs: encode.clone(),
            })
            .collect(),
        api @ ("qsv" | "vaapi") => {
            let device = if settings.hardware_device_path.is_empty() {
                default_intel_device(api).to_owned()
            } else {
                settings.hardware_device_path.clone()
            };
            vec![Target {
                id: format!("{api}:{device}"),
                api: api.into(),
                device,
                name: String::new(),
                weight: 1,
                max_sessions: 0,
                decode_codecs: settings.hardware_decoding_codecs.clone(),
                encode_codecs: encode,
            }]
        }
        "pool" => settings
            .hardware_pool
            .iter()
            .filter(|d| d.enabled && (!d.decode_codecs.is_empty() || !d.encode_codecs.is_empty()))
            .map(|d| Target {
                id: d.id(),
                api: d.api.clone(),
                device: d.device.clone(),
                name: d.name.clone(),
                weight: d.weight.max(1),
                max_sessions: d.max_sessions,
                decode_codecs: d.decode_codecs.clone(),
                encode_codecs: d.encode_codecs.clone(),
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The settings one session runs with on `target`: the pool resolved to
/// one API and device, with that device's codecs.
pub fn apply(settings: &PlaybackSettings, target: &Target) -> PlaybackSettings {
    let mut s = settings.clone();
    s.hardware_acceleration = target.api.clone();
    if target.api == "nvenc" {
        s.hardware_device_index = target
            .device
            .parse()
            .unwrap_or(settings.hardware_device_index);
    } else {
        s.hardware_device_path = target.device.clone();
    }
    s.hardware_decoding_codecs = target.decode_codecs.clone();
    s.hardware_encoding = !target.encode_codecs.is_empty();
    s.hardware_encode_codecs = target.encode_codecs.clone();
    s
}

/// The settings of a session no device is free for: the CPU does it.
pub fn without_hardware(settings: &PlaybackSettings) -> PlaybackSettings {
    let mut s = settings.clone();
    s.hardware_acceleration = "none".into();
    s
}

/// Chooses among `targets` (pure). Devices at their `max_sessions` are
/// skipped. The rest compare by load per share, `(sessions + 1) /
/// weight`: with weights 2 (NVIDIA) and 1 (Intel) three sessions go
/// NVIDIA, NVIDIA, Intel. Ties: more free video memory (NVIDIA, in
/// [`MEMORY_STEP_MB`] steps), then round robin from `rotation`.
pub fn pick<'a>(
    targets: &'a [Target],
    active: &HashMap<String, usize>,
    gpus: &[NvidiaGpu],
    rotation: usize,
) -> Option<&'a Target> {
    let n = targets.len().max(1);
    let sessions = |t: &Target| active.get(&t.id).copied().unwrap_or(0);
    let free_steps = |t: &Target| {
        if t.api != "nvenc" {
            return 0;
        }
        let index: Option<u32> = t.device.parse().ok();
        gpus.iter()
            .find(|g| Some(g.index) == index)
            .and_then(NvidiaGpu::memory_free_mb)
            .map_or(0, |mb| mb / MEMORY_STEP_MB)
    };
    (0..targets.len())
        .map(|i| &targets[(rotation + i) % n])
        .filter(|t| t.max_sessions == 0 || sessions(t) < t.max_sessions as usize)
        .min_by(|a, b| {
            // (sa + 1) / wa vs (sb + 1) / wb without fractions.
            let left = (sessions(a) as u64 + 1) * u64::from(b.weight.max(1));
            let right = (sessions(b) as u64 + 1) * u64::from(a.weight.max(1));
            left.cmp(&right)
                .then_with(|| free_steps(b).cmp(&free_steps(a)))
        })
}

static ROTATION: AtomicUsize = AtomicUsize::new(0);
/// Sessions that got a device but are not registered yet.
static PENDING: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Default::default);

/// A device promised to a session that is starting; counts as one of its
/// sessions until dropped (when the session is registered, or failed).
#[derive(Debug)]
pub struct Reservation {
    id: String,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = pending.get_mut(&self.id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                pending.remove(&self.id);
            }
        }
    }
}

/// Sessions per device: running ones plus those starting.
pub fn load() -> HashMap<String, usize> {
    let mut counts = crate::session::target_sessions();
    for (id, n) in PENDING.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        *counts.entry(id.clone()).or_insert(0) += n;
    }
    counts
}

fn uses_nvidia(settings: &PlaybackSettings) -> bool {
    match settings.hardware_acceleration.as_str() {
        "nvenc" => settings.gpu_load_balancing,
        "pool" => settings
            .hardware_pool
            .iter()
            .any(|d| d.enabled && d.api == "nvenc"),
        _ => false,
    }
}

/// The devices of these settings with `nvidia-smi`'s view (live status).
pub async fn configured_targets(settings: &PlaybackSettings) -> (Vec<Target>, Vec<NvidiaGpu>) {
    let gpus = if uses_nvidia(settings) {
        nvidia_gpus().await
    } else {
        Vec::new()
    };
    (targets(settings, &gpus), gpus)
}

/// Picks and reserves a device for a session about to start. `None`: no
/// hardware is configured, or every device is at its limit.
pub async fn reserve(settings: &PlaybackSettings) -> Option<(Target, Reservation)> {
    let (list, gpus) = configured_targets(settings).await;
    if list.is_empty() {
        return None;
    }
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let mut active = crate::session::target_sessions();
    for (id, n) in pending.iter() {
        *active.entry(id.clone()).or_insert(0) += n;
    }
    let target = pick(
        &list,
        &active,
        &gpus,
        ROTATION.fetch_add(1, Ordering::Relaxed),
    )?
    .clone();
    *pending.entry(target.id.clone()).or_insert(0) += 1;
    tracing::debug!(device = %target.id, ?active, "transcode device reserved");
    Some((target.clone(), Reservation { id: target.id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMI: &str = "0, NVIDIA GeForce RTX 3060, 00000000:01:00.0, 560.35.03, 12288, 1024, 7, 1\n\
                       1, NVIDIA RTX A2000, 00000000:41:00.0, 560.35.03, 6144, 5900, 3, 0\n\
                       2, Tesla P4, 00000000:81:00.0, 560.35.03, 7680, [N/A], [Not Supported], [N/A]\n\
                       garbage line\n";

    #[test]
    fn parses_nvidia_smi() {
        let gpus = parse_nvidia_smi(SMI);
        assert_eq!(gpus.len(), 3);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 3060");
        assert_eq!(gpus[0].memory_free_mb(), Some(11264));
        assert_eq!(gpus[1].pci_bus_id.as_deref(), Some("00000000:41:00.0"));
        assert_eq!(
            (
                gpus[2].memory_used_mb,
                gpus[2].utilization_percent,
                gpus[2].memory_free_mb()
            ),
            (None, None, None)
        );
        assert!(parse_nvidia_smi("").is_empty());
    }

    fn settings(balancing: bool, devices: Vec<u32>, index: u32) -> PlaybackSettings {
        PlaybackSettings {
            gpu_load_balancing: balancing,
            hardware_devices: devices,
            hardware_device_index: index,
            ..Default::default()
        }
    }

    #[test]
    fn candidate_devices() {
        let gpus = parse_nvidia_smi(SMI);
        assert_eq!(candidates(&settings(true, vec![], 0), &gpus), [0, 1, 2]);
        assert_eq!(candidates(&settings(true, vec![2, 0], 0), &gpus), [2, 0]);
        assert_eq!(candidates(&settings(false, vec![2, 0], 1), &gpus), [1]);
        assert_eq!(
            candidates(&settings(true, vec![], 3), &[]),
            [3],
            "no nvidia-smi: the configured GPU"
        );
    }

    fn target(id: &str, weight: u32, max: u32) -> Target {
        let (api, device) = id.split_once(':').unwrap();
        Target {
            id: id.into(),
            api: api.into(),
            device: device.into(),
            name: String::new(),
            weight,
            max_sessions: max,
            decode_codecs: vec![],
            encode_codecs: vec!["h264".into()],
        }
    }

    #[test]
    fn spreads_sessions_over_cards() {
        let list = [target("nvenc:0", 1, 0), target("nvenc:1", 1, 0)];
        let mut active = HashMap::new();
        let id = |t: Option<&Target>| t.map(|t| t.id.clone()).unwrap_or_default();
        // Equal load and no memory data: round robin.
        assert_eq!(id(pick(&list, &active, &[], 0)), "nvenc:0");
        assert_eq!(id(pick(&list, &active, &[], 1)), "nvenc:1");
        // Fewest sessions wins whatever the rotation.
        active.insert("nvenc:0".to_owned(), 1);
        assert_eq!(id(pick(&list, &active, &[], 0)), "nvenc:1");
        active.insert("nvenc:1".to_owned(), 1);
        // Equal sessions: the card with more free memory (card 1 is nearly full).
        let gpus = parse_nvidia_smi(SMI);
        assert_eq!(id(pick(&list, &active, &gpus, 1)), "nvenc:0");
        active.insert("nvenc:0".to_owned(), 3);
        assert_eq!(
            id(pick(&list, &active, &gpus, 0)),
            "nvenc:1",
            "load beats memory"
        );
    }

    #[test]
    fn mixed_architectures_by_weight_and_limit() {
        // NVIDIA with share 2, Intel Quick Sync with share 1.
        let list = [
            target("nvenc:0", 2, 0),
            target("qsv:/dev/dri/renderD128", 1, 0),
        ];
        let gpus = parse_nvidia_smi(SMI);
        let mut active: HashMap<String, usize> = HashMap::new();
        let mut order = Vec::new();
        for i in 0..6 {
            let chosen = pick(&list, &active, &gpus, i).unwrap().id.clone();
            *active.entry(chosen.clone()).or_insert(0) += 1;
            order.push(chosen);
        }
        assert_eq!(active["nvenc:0"], 4);
        assert_eq!(active["qsv:/dev/dri/renderD128"], 2);
        assert_eq!(
            order[..3],
            ["nvenc:0", "nvenc:0", "qsv:/dev/dri/renderD128"]
        );
        // Limits: the Intel GPU takes one session, the NVIDIA card two; then the CPU.
        let limited = [
            target("nvenc:0", 2, 2),
            target("qsv:/dev/dri/renderD128", 1, 1),
        ];
        let mut active: HashMap<String, usize> = HashMap::new();
        for _ in 0..3 {
            let chosen = pick(&limited, &active, &gpus, 0).unwrap().id.clone();
            *active.entry(chosen).or_insert(0) += 1;
        }
        assert_eq!(
            (active["nvenc:0"], active["qsv:/dev/dri/renderD128"]),
            (2, 1)
        );
        assert!(
            pick(&limited, &active, &gpus, 0).is_none(),
            "full pool: CPU"
        );
    }

    #[test]
    fn settings_to_targets() {
        use lisek_core::playback_settings::HardwareDevice;
        let gpus = parse_nvidia_smi(SMI);
        let nvenc = PlaybackSettings {
            hardware_acceleration: "nvenc".into(),
            ..Default::default()
        };
        let list = targets(&nvenc, &gpus);
        assert_eq!(
            list.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["nvenc:0", "nvenc:1", "nvenc:2"]
        );
        assert_eq!(list[0].name, "NVIDIA GeForce RTX 3060");
        let pool = PlaybackSettings {
            hardware_acceleration: "pool".into(),
            hardware_pool: vec![
                HardwareDevice {
                    api: "nvenc".into(),
                    device: "1".into(),
                    enabled: true,
                    weight: 2,
                    ..Default::default()
                },
                HardwareDevice {
                    api: "qsv".into(),
                    device: "/dev/dri/renderD128".into(),
                    enabled: true,
                    decode_codecs: vec!["h264".into()],
                    encode_codecs: vec!["h264".into()],
                    ..Default::default()
                },
                HardwareDevice {
                    api: "vaapi".into(),
                    device: "/dev/dri/renderD129".into(),
                    enabled: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let list = targets(&pool, &gpus);
        assert_eq!(list.len(), 2, "disabled devices are left out");
        let qsv = apply(&pool, &list[1]);
        assert_eq!(
            (
                qsv.hardware_acceleration.as_str(),
                qsv.hardware_device_path.as_str(),
                qsv.hardware_encode_codecs.len()
            ),
            ("qsv", "/dev/dri/renderD128", 1)
        );
        assert_eq!(qsv.hardware_decoding_codecs, ["h264"]);
        let nv = apply(&pool, &list[0]);
        assert_eq!(
            (nv.hardware_acceleration.as_str(), nv.hardware_device_index),
            ("nvenc", 1)
        );
        assert!(
            nv.hardware_decoding_codecs.is_empty(),
            "the pool entry decodes nothing on the GPU"
        );
        assert_eq!(without_hardware(&pool).hardware_acceleration, "none");
        let single = PlaybackSettings {
            hardware_acceleration: "vaapi".into(),
            ..Default::default()
        };
        assert_eq!(targets(&single, &[])[0].id, "vaapi:/dev/dri/renderD128");
        assert!(targets(&PlaybackSettings::default(), &gpus).is_empty());
    }
}
