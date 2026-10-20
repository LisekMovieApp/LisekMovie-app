//! Playback: what to send each client (direct play, remux, transcode),
//! the FFmpeg command lines for it, and the HLS session engine.
//!
//! Independent of the HTTP framework and the database: the server feeds
//! in stored probes and settings and serves what [`session`] produces.
pub mod command;
pub mod decision;
pub mod downmix;
pub mod filters;
pub mod fmp4;
pub mod hls;
pub mod keyframes;
pub mod profile;
pub mod session;
pub mod subtitles;
pub mod tonemap;
pub mod transcode;
pub mod trickplay;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

/// A single inclusive HTTP range. Multiple ranges are deliberately unsupported.
pub fn parse_range(header: &str, length: u64) -> Option<ByteRange> {
    let value = header.strip_prefix("bytes=")?;
    let (start, end) = value.split_once('-')?;
    if length == 0 || value.contains(',') { return None; }
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        return (suffix > 0).then_some(ByteRange {
            start: length.saturating_sub(suffix), end: length - 1,
        });
    }
    let start: u64 = start.parse().ok()?;
    let end = if end.is_empty() { length - 1 } else { end.parse::<u64>().ok()?.min(length - 1) };
    (start < length && start <= end).then_some(ByteRange { start, end })
}

pub fn content_type(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" | "ogg" => "video/ogg",
        "mkv" => "video/x-matroska",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "ts" | "m2ts" => "video/mp2t",
        _ => "application/octet-stream",
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TranscodeSafety {
    pub enabled: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub max_bitrate_kbps: Option<u32>,
}

impl Default for TranscodeSafety {
    fn default() -> Self {
        Self { enabled: true, max_width: 1920, max_height: 1080, max_bitrate_kbps: None }
    }
}

impl TranscodeSafety {
    pub fn rejection(&self, width: u32, height: u32, bitrate_kbps: u32) -> Option<&'static str> {
        if !self.enabled { return None; }
        if width > self.max_width || height > self.max_height { return Some("resolution_exceeded"); }
        if self.max_bitrate_kbps.is_some_and(|limit| bitrate_kbps > limit) { return Some("bitrate_exceeded"); }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_seek_open_ended_and_suffix_ranges() {
        assert_eq!(parse_range("bytes=10-19", 100), Some(ByteRange { start: 10, end: 19 }));
        assert_eq!(parse_range("bytes=90-", 100), Some(ByteRange { start: 90, end: 99 }));
        assert_eq!(parse_range("bytes=-10", 100), Some(ByteRange { start: 90, end: 99 }));
        assert_eq!(parse_range("bytes=-999", 100), Some(ByteRange { start: 0, end: 99 }));
        assert_eq!(parse_range("bytes=90-999", 100), Some(ByteRange { start: 90, end: 99 }));
    }

    #[test]
    fn rejects_invalid_unsatisfiable_and_multiple_ranges() {
        for range in ["bytes=100-", "bytes=20-10", "bytes=-0", "bytes=", "bytes=a-b", "bytes=0-1,4-5", "items=0-2"] {
            assert_eq!(parse_range(range, 100), None, "{range}");
        }
        assert_eq!(parse_range("bytes=0-", 0), None);
    }

    #[test]
    fn safety_limits_include_boundaries_and_can_be_disabled() {
        let mut safety = TranscodeSafety::default();
        assert_eq!(safety.rejection(1920, 1080, 100_000), None);
        assert_eq!(safety.rejection(3840, 2160, 10_000), Some("resolution_exceeded"));
        safety.max_bitrate_kbps = Some(10_000);
        assert_eq!(safety.rejection(1280, 720, 10_001), Some("bitrate_exceeded"));
        safety.enabled = false;
        assert_eq!(safety.rejection(3840, 2160, 100_000), None);
    }
}
