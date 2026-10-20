//! HDR to SDR tone mapping (BT.2390 by default) and where it runs.
//!
//! | backend      | needs                              | frames            |
//! |--------------|------------------------------------|-------------------|
//! | `cuda`       | jellyfin-ffmpeg `tonemap_cuda`     | stay in VRAM      |
//! | `libplacebo` | FFmpeg with libplacebo + Vulkan    | RAM -> Vulkan     |
//! | `opencl`     | `tonemap_opencl` + OpenCL driver   | RAM -> OpenCL     |
//! | `software`   | `zscale` + `tonemap`               | CPU (slow at 4K)  |
//!
//! With NVDEC + `tonemap_cuda` + NVENC a 4K HDR movie never leaves the
//! GPU: decode, scale, tone map and encode all run there.

use crate::decision::TonemapPlan;
use crate::filters::{Frames, VideoChain};
use crate::transcode::FfmpegTools;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TonemapBackend {
    Cuda,
    Libplacebo,
    Opencl,
    Software,
}

impl TonemapBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            TonemapBackend::Cuda => "cuda",
            TonemapBackend::Libplacebo => "libplacebo",
            TonemapBackend::Opencl => "opencl",
            TonemapBackend::Software => "software",
        }
    }

    fn supported(self, tools: &FfmpegTools) -> bool {
        match self {
            TonemapBackend::Cuda => tools.has_filter("tonemap_cuda") && tools.has_filter("scale_cuda"),
            TonemapBackend::Libplacebo => tools.has_filter("libplacebo"),
            TonemapBackend::Opencl => tools.has_filter("tonemap_opencl"),
            TonemapBackend::Software => tools.has_filter("zscale") && tools.has_filter("tonemap"),
        }
    }
}

/// Backends usable with this build, fastest first. `nvidia` and `vulkan`
/// are the results of the device tests in [`crate::transcode::capabilities`].
pub fn available_backends(tools: &FfmpegTools, nvidia: bool, vulkan: bool) -> Vec<TonemapBackend> {
    [
        (TonemapBackend::Cuda, nvidia),
        (TonemapBackend::Libplacebo, vulkan),
        (TonemapBackend::Opencl, nvidia),
        (TonemapBackend::Software, true),
    ]
    .into_iter()
    .filter(|(backend, device)| *device && backend.supported(tools))
    .map(|(backend, _)| backend)
    .collect()
}

/// The backend for the `tonemapping_backend` setting. `auto` prefers the
/// GPU ones only when NVIDIA acceleration is configured. An explicit
/// choice the build cannot do falls back to `auto`.
pub fn select(setting: &str, tools: &FfmpegTools, gpu: bool) -> Option<TonemapBackend> {
    let explicit = match setting {
        "cuda" => Some(TonemapBackend::Cuda),
        "libplacebo" => Some(TonemapBackend::Libplacebo),
        "opencl" => Some(TonemapBackend::Opencl),
        "software" => Some(TonemapBackend::Software),
        _ => None,
    };
    if let Some(backend) = explicit.filter(|b| b.supported(tools)) {
        return Some(backend);
    }
    let order: &[TonemapBackend] = if gpu {
        &[TonemapBackend::Cuda, TonemapBackend::Libplacebo, TonemapBackend::Opencl, TonemapBackend::Software]
    } else {
        &[TonemapBackend::Software]
    };
    order.iter().copied().find(|b| b.supported(tools))
}

/// The configured algorithm in the backend's spelling. Upstream
/// `tonemap_opencl` and `tonemap` have no BT.2390 and use Hable instead
/// (jellyfin-ffmpeg's `tonemap_opencl` has it).
pub fn algorithm_name(backend: TonemapBackend, configured: &str, tools: &FfmpegTools) -> String {
    match (backend, configured) {
        (TonemapBackend::Libplacebo, "bt2390") => "bt.2390".into(),
        (TonemapBackend::Opencl, "bt2390") if tools.jellyfin_build => "bt2390".into(),
        (TonemapBackend::Opencl | TonemapBackend::Software, "bt2390") => "hable".into(),
        (_, other) => other.into(),
    }
}

/// Appends tone mapping plus scaling to `width`x`height` to `chain`. The
/// result is 8-bit BT.709 (`nv12` on CUDA, `yuv420p` in RAM).
pub fn apply(chain: &mut VideoChain, plan: &TonemapPlan, width: u32, height: u32) {
    // Many remuxes lack colour tags; the tone mappers refuse untagged
    // input, so state what the stream is.
    let tag = format!(
        "setparams=color_primaries=bt2020:color_trc={}:colorspace=bt2020nc",
        plan.transfer
    );
    let mut params = format!("peak={}:desat={}", plan.peak, plan.desat);
    if let Some(param) = plan.param {
        params.push_str(&format!(":param={param}"));
    }
    match plan.backend {
        TonemapBackend::Cuda => {
            chain.cuda(&tag, None);
            chain.cuda(format!("scale_cuda=w={width}:h={height}:interp_algo=bicubic"), None);
            let dovi = if plan.dolby_vision { ":apply_dovi=1" } else { "" };
            chain.cuda(
                format!(
                    "tonemap_cuda=format=nv12:p=bt709:t=bt709:m=bt709:tonemap={}:{params}{dovi}",
                    plan.algorithm
                ),
                Some("nv12"),
            );
        }
        TonemapBackend::Libplacebo => {
            if chain.frames().is_cuda() {
                // Shrink in VRAM before the round trip through RAM.
                chain.cuda(format!("scale_cuda=w={width}:h={height}"), None);
            }
            chain.cpu(&tag);
            chain.vulkan_upload();
            let dovi = if plan.dolby_vision { "1" } else { "0" };
            chain.vulkan(format!(
                "libplacebo=w={width}:h={height}:format=yuv420p:colorspace=bt709:\
                 color_primaries=bt709:color_trc=bt709:range=tv:tonemapping={}:\
                 peak_detect=1:apply_dolbyvision={dovi}",
                plan.algorithm
            ));
            chain.vulkan_download("yuv420p");
        }
        TonemapBackend::Opencl => {
            if chain.frames().is_cuda() {
                chain.cuda(format!("scale_cuda=w={width}:h={height}"), None);
            } else {
                chain.cpu(format!("scale=w={width}:h={height}"));
            }
            chain.cpu(&tag);
            chain.cpu("format=p010le");
            chain.opencl_upload();
            chain.opencl(format!(
                "tonemap_opencl=tonemap={}:{params}:t=bt709:m=bt709:p=bt709:format=nv12",
                plan.algorithm
            ));
            chain.opencl_download("nv12");
        }
        TonemapBackend::Software => {
            chain.cpu(format!(
                "zscale=w={width}:h={height}:f=bicubic:tin={}:min=bt2020nc:pin=bt2020:rin=tv:t=linear:npl=100",
                plan.transfer
            ));
            chain.cpu("format=gbrpf32le");
            chain.cpu("zscale=p=bt709");
            chain.cpu(format!("tonemap=tonemap={}:{params}", plan.algorithm));
            chain.cpu("zscale=t=bt709:m=bt709:r=tv");
            chain.cpu("format=yuv420p");
        }
    }
    debug_assert!(!matches!(chain.frames(), Frames::Vulkan | Frames::Opencl));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(filters: &[&str], jellyfin: bool) -> FfmpegTools {
        FfmpegTools {
            jellyfin_build: jellyfin,
            filters: filters.iter().map(|f| f.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn auto_prefers_the_gpu_and_falls_back() {
        let all = tools(&["tonemap_cuda", "scale_cuda", "libplacebo", "tonemap_opencl", "zscale", "tonemap"], true);
        assert_eq!(select("auto", &all, true), Some(TonemapBackend::Cuda));
        assert_eq!(select("auto", &all, false), Some(TonemapBackend::Software));
        let upstream = tools(&["scale_cuda", "libplacebo", "tonemap_opencl", "zscale", "tonemap"], false);
        assert_eq!(select("auto", &upstream, true), Some(TonemapBackend::Libplacebo));
        assert_eq!(select("cuda", &upstream, true), Some(TonemapBackend::Libplacebo), "falls back");
        assert_eq!(select("opencl", &upstream, true), Some(TonemapBackend::Opencl));
        assert_eq!(select("auto", &tools(&[], false), true), None);
    }

    #[test]
    fn bt2390_is_spelled_per_backend() {
        let upstream = tools(&[], false);
        assert_eq!(algorithm_name(TonemapBackend::Libplacebo, "bt2390", &upstream), "bt.2390");
        assert_eq!(algorithm_name(TonemapBackend::Cuda, "bt2390", &upstream), "bt2390");
        assert_eq!(algorithm_name(TonemapBackend::Opencl, "bt2390", &upstream), "hable");
        assert_eq!(algorithm_name(TonemapBackend::Opencl, "bt2390", &tools(&[], true)), "bt2390");
        assert_eq!(algorithm_name(TonemapBackend::Software, "mobius", &upstream), "mobius");
    }

    #[test]
    fn listed_backends_need_their_device() {
        let all = tools(&["tonemap_cuda", "scale_cuda", "libplacebo", "zscale", "tonemap"], true);
        assert_eq!(
            available_backends(&all, true, false),
            [TonemapBackend::Cuda, TonemapBackend::Software]
        );
        assert_eq!(
            available_backends(&all, false, true),
            [TonemapBackend::Libplacebo, TonemapBackend::Software]
        );
    }
}
