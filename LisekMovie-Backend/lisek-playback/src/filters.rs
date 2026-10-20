//! A linear FFmpeg video filter chain that tracks where frames live
//! (system memory, CUDA, Vulkan, OpenCL) and inserts the uploads and
//! downloads between filters, so a chain is written as a list of steps.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frames {
    Cpu,
    /// In CUDA memory, `sw` being the layout `hwdownload` must name.
    Cuda { sw: &'static str },
    Vulkan,
    Opencl,
}

impl Frames {
    pub fn is_cuda(self) -> bool {
        matches!(self, Frames::Cuda { .. })
    }
}

#[derive(Debug, Clone)]
pub struct VideoChain {
    filters: Vec<String>,
    frames: Frames,
    ten_bit: bool,
    device: u32,
    pub uses_cuda: bool,
    pub uses_vulkan: bool,
    pub uses_opencl: bool,
}

impl VideoChain {
    /// `ten_bit`: the frames are 10-bit until a step makes them 8-bit.
    /// `device`: CUDA device index for uploads.
    pub fn new(frames: Frames, ten_bit: bool, device: u32) -> Self {
        Self {
            filters: vec![],
            frames,
            ten_bit,
            device,
            uses_cuda: frames.is_cuda(),
            uses_vulkan: false,
            uses_opencl: false,
        }
    }

    pub fn frames(&self) -> Frames {
        self.frames
    }

    pub fn set_eight_bit(&mut self) {
        self.ten_bit = false;
    }

    fn push(&mut self, filter: impl AsRef<str>) {
        self.filters.push(filter.as_ref().to_owned());
    }

    fn cuda_layout(&self) -> &'static str {
        if self.ten_bit { "p010le" } else { "nv12" }
    }

    /// Brings frames to system memory.
    pub fn to_cpu(&mut self) {
        match self.frames {
            Frames::Cpu => {}
            Frames::Cuda { sw } => {
                self.push("hwdownload");
                self.push(format!("format={sw}"));
                self.frames = Frames::Cpu;
            }
            Frames::Vulkan | Frames::Opencl => {
                unreachable!("Vulkan/OpenCL steps end with their own download")
            }
        }
    }

    /// Brings frames to CUDA memory.
    pub fn to_cuda(&mut self) {
        if self.frames == Frames::Cpu {
            let layout = self.cuda_layout();
            self.push(format!("format={layout}"));
            self.push(format!("hwupload_cuda=device={}", self.device));
            self.frames = Frames::Cuda { sw: layout };
            self.uses_cuda = true;
        }
    }

    /// A filter working on frames in system memory.
    pub fn cpu(&mut self, filter: impl AsRef<str>) {
        self.to_cpu();
        self.push(filter);
    }

    /// A filter working on CUDA frames; `out` is the layout it produces
    /// when it changes it (`tonemap_cuda=format=nv12`).
    pub fn cuda(&mut self, filter: impl AsRef<str>, out: Option<&'static str>) {
        self.to_cuda();
        self.push(filter);
        if let Some(sw) = out {
            self.frames = Frames::Cuda { sw };
            self.ten_bit = sw.contains("10");
        }
    }

    pub fn vulkan_upload(&mut self) {
        self.to_cpu();
        self.push("hwupload");
        self.frames = Frames::Vulkan;
        self.uses_vulkan = true;
    }

    pub fn vulkan(&mut self, filter: impl AsRef<str>) {
        debug_assert_eq!(self.frames, Frames::Vulkan);
        self.push(filter);
    }

    pub fn vulkan_download(&mut self, sw: &str) {
        self.push("hwdownload");
        self.push(format!("format={sw}"));
        self.frames = Frames::Cpu;
    }

    pub fn opencl_upload(&mut self) {
        self.to_cpu();
        self.push("hwupload");
        self.frames = Frames::Opencl;
        self.uses_opencl = true;
    }

    pub fn opencl(&mut self, filter: impl AsRef<str>) {
        debug_assert_eq!(self.frames, Frames::Opencl);
        self.push(filter);
    }

    pub fn opencl_download(&mut self, sw: &str) {
        self.push("hwdownload");
        self.push(format!("format={sw}"));
        self.frames = Frames::Cpu;
    }

    pub fn is_empty(&self) -> bool {
        self.filters.is_empty()
    }

    pub fn to_filter_string(&self) -> String {
        if self.filters.is_empty() {
            "null".into()
        } else {
            self.filters.join(",")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_transfers_between_memory_kinds() {
        let mut chain = VideoChain::new(Frames::Cuda { sw: "p010le" }, true, 1);
        chain.cuda("scale_cuda=w=1920:h=1080", None);
        chain.cpu("subtitles=filename=a.srt");
        chain.cuda("scale_cuda=w=1280:h=720", Some("nv12"));
        assert_eq!(
            chain.to_filter_string(),
            "scale_cuda=w=1920:h=1080,hwdownload,format=p010le,subtitles=filename=a.srt,\
             format=p010le,hwupload_cuda=device=1,scale_cuda=w=1280:h=720"
        );
        assert_eq!(chain.frames(), Frames::Cuda { sw: "nv12" });
        chain.to_cpu();
        assert!(chain.to_filter_string().ends_with("hwdownload,format=nv12"));
    }

    #[test]
    fn vulkan_round_trip_returns_to_system_memory() {
        let mut chain = VideoChain::new(Frames::Cpu, true, 0);
        chain.vulkan_upload();
        chain.vulkan("libplacebo=w=1280:h=720");
        chain.vulkan_download("yuv420p");
        assert_eq!(chain.frames(), Frames::Cpu);
        assert!(chain.uses_vulkan && !chain.uses_cuda);
        assert_eq!(chain.to_filter_string(), "hwupload,libplacebo=w=1280:h=720,hwdownload,format=yuv420p");
    }
}
