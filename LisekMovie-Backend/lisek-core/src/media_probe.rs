//! Technical facts about one media file, as reported by `ffprobe`.
//!
//! Produced by `lisek-playback` (which runs ffprobe), persisted by
//! `lisek-db` (`media_streams` table), and read back by the playback
//! endpoints so a file is probed once per change instead of once per
//! request.

use serde::Serialize;

/// The container-level facts plus every stream ffprobe reported.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct MediaProbe {
    /// ffprobe's `format_name`, e.g. `"matroska,webm"`.
    pub container: Option<String>,
    pub duration_seconds: Option<f64>,
    pub bitrate_kbps: Option<i32>,
    /// ffprobe's `format.start_time`: the timestamp playback time zero
    /// corresponds to. Near zero for Matroska/MP4, often hundreds of
    /// seconds for MPEG-TS/M2TS. `None` for probes stored before 0015.
    pub start_time_seconds: Option<f64>,
    pub streams: Vec<MediaStream>,
    /// Chapters (scenes) from the container, in playback time (the
    /// container's `start_time` already subtracted). Empty when the file
    /// has none, and on probes read back for playback.
    pub chapters: Vec<MediaChapter>,
}

/// One chapter of a Matroska/MP4 file (`ffprobe -show_chapters`).
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct MediaChapter {
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
    /// The container's `title` tag, e.g. "Chapter 3" or a scene name.
    pub title: Option<String>,
}

/// One elementary stream. Fields that do not apply to a stream's kind
/// (e.g. `channels` on a video stream) are `None`.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct MediaStream {
    /// ffprobe's own stream index, stable for a given file.
    pub index: i32,
    pub kind: StreamKind,
    pub codec: Option<String>,
    pub profile: Option<String>,
    /// ISO 639-2 tag from the container, e.g. `"pol"`.
    pub language: Option<String>,
    pub title: Option<String>,
    pub is_default: bool,
    pub is_forced: bool,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub bit_depth: Option<i32>,
    pub pixel_format: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    pub color_space: Option<String>,
    pub field_order: Option<String>,
    pub frame_rate: Option<f64>,
    pub channels: Option<i32>,
    pub channel_layout: Option<String>,
    pub sample_rate: Option<i32>,
    pub bitrate_kbps: Option<i32>,
    /// Dolby Vision profile from the stream's DOVI configuration record
    /// (5, 7, 8 ...), `None` for streams without Dolby Vision.
    pub dovi_profile: Option<i32>,
}

/// The high dynamic range format of a video stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HdrFormat {
    /// PQ transfer (HDR10, HDR10+).
    Hdr10,
    /// Hybrid log-gamma.
    Hlg,
    /// Dolby Vision. Profiles 7 and 8 carry an HDR10-compatible base
    /// layer; profile 5 does not, and only a Dolby Vision aware decoder or
    /// tone mapper shows correct colours.
    DolbyVision { profile: i32 },
}

impl MediaStream {
    /// `None` for SDR video and for non-video streams.
    pub fn hdr_format(&self) -> Option<HdrFormat> {
        if self.kind != StreamKind::Video {
            return None;
        }
        if let Some(profile) = self.dovi_profile {
            return Some(HdrFormat::DolbyVision { profile });
        }
        match self.color_transfer.as_deref() {
            Some("smpte2084") => Some(HdrFormat::Hdr10),
            Some("arib-std-b67") => Some(HdrFormat::Hlg),
            _ => None,
        }
    }

    /// Bits per sample, from the probe or, when ffprobe left it out, from
    /// the pixel format name (`yuv420p10le` -> 10).
    pub fn effective_bit_depth(&self) -> u32 {
        if let Some(depth) = self.bit_depth.and_then(|d| u32::try_from(d).ok()).filter(|d| *d > 0) {
            return depth;
        }
        let format = self.pixel_format.as_deref().unwrap_or_default();
        if format.contains("12") {
            12
        } else if format.contains("10") || format.starts_with("p010") {
            10
        } else {
            8
        }
    }

    /// Interlaced field order reported by ffprobe.
    pub fn is_interlaced(&self) -> bool {
        matches!(self.field_order.as_deref(), Some("tt" | "bb" | "tb" | "bt"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Attachment,
    Data,
}

impl StreamKind {
    /// The value stored in `media_streams.stream_type`.
    pub fn as_db_str(self) -> &'static str {
        match self {
            StreamKind::Video => "video",
            StreamKind::Audio => "audio",
            StreamKind::Subtitle => "subtitle",
            StreamKind::Attachment => "attachment",
            StreamKind::Data => "data",
        }
    }

    /// Accepts both ffprobe's `codec_type` and the stored DB value.
    /// Unknown kinds are treated as `Data`.
    pub fn parse(value: &str) -> Self {
        match value {
            "video" => StreamKind::Video,
            "audio" => StreamKind::Audio,
            "subtitle" => StreamKind::Subtitle,
            "attachment" => StreamKind::Attachment,
            _ => StreamKind::Data,
        }
    }
}

impl MediaProbe {
    /// The first video stream that is not an embedded cover image.
    pub fn primary_video(&self) -> Option<&MediaStream> {
        self.streams
            .iter()
            .filter(|s| s.kind == StreamKind::Video)
            .find(|s| !is_cover_art(s))
    }

    /// The default audio stream, or the first one when none is flagged.
    pub fn primary_audio(&self) -> Option<&MediaStream> {
        let mut audio = self.streams.iter().filter(|s| s.kind == StreamKind::Audio);
        let first = audio.clone().next();
        audio.find(|s| s.is_default).or(first)
    }

    /// The stream with ffprobe index `index`.
    pub fn stream(&self, index: i32) -> Option<&MediaStream> {
        self.streams.iter().find(|s| s.index == index)
    }

    /// The audio stream a player picks when it plays the file directly:
    /// the first one in file order.
    pub fn first_audio(&self) -> Option<&MediaStream> {
        self.streams.iter().find(|s| s.kind == StreamKind::Audio)
    }
}

/// MKV/MP4 files often carry a poster as a one-frame "video" stream.
fn is_cover_art(stream: &MediaStream) -> bool {
    matches!(stream.codec.as_deref(), Some("mjpeg" | "png" | "bmp" | "gif"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(index: i32, kind: StreamKind, codec: &str) -> MediaStream {
        MediaStream {
            index,
            kind,
            codec: Some(codec.into()),
            profile: None,
            language: None,
            title: None,
            is_default: false,
            is_forced: false,
            width: None,
            height: None,
            bit_depth: None,
            pixel_format: None,
            color_transfer: None,
            color_primaries: None,
            color_space: None,
            field_order: None,
            frame_rate: None,
            channels: None,
            channel_layout: None,
            sample_rate: None,
            bitrate_kbps: None,
            dovi_profile: None,
        }
    }

    #[test]
    fn primary_video_skips_embedded_cover_art() {
        let probe = MediaProbe {
            container: None,
            duration_seconds: None,
            bitrate_kbps: None,
            start_time_seconds: None,
            chapters: Vec::new(),
            streams: vec![stream(0, StreamKind::Video, "mjpeg"), stream(1, StreamKind::Video, "hevc")],
        };
        assert_eq!(probe.primary_video().unwrap().index, 1);
    }

    #[test]
    fn primary_audio_prefers_the_default_flag() {
        let mut second = stream(2, StreamKind::Audio, "ac3");
        second.is_default = true;
        let probe = MediaProbe {
            container: None,
            duration_seconds: None,
            bitrate_kbps: None,
            start_time_seconds: None,
            chapters: Vec::new(),
            streams: vec![stream(1, StreamKind::Audio, "aac"), second],
        };
        assert_eq!(probe.primary_audio().unwrap().index, 2);
    }

    #[test]
    fn hdr_format_and_bit_depth_come_from_the_probe() {
        let mut video = stream(0, StreamKind::Video, "hevc");
        assert_eq!(video.hdr_format(), None);
        video.color_transfer = Some("smpte2084".into());
        assert_eq!(video.hdr_format(), Some(HdrFormat::Hdr10));
        video.dovi_profile = Some(8);
        assert_eq!(video.hdr_format(), Some(HdrFormat::DolbyVision { profile: 8 }));
        video.pixel_format = Some("yuv420p10le".into());
        assert_eq!(video.effective_bit_depth(), 10);
        video.bit_depth = Some(12);
        assert_eq!(video.effective_bit_depth(), 12);
        let mut audio = stream(1, StreamKind::Audio, "aac");
        audio.color_transfer = Some("smpte2084".into());
        assert_eq!(audio.hdr_format(), None);
    }

    #[test]
    fn stream_kind_round_trips_and_defaults_to_data() {
        for kind in [StreamKind::Video, StreamKind::Audio, StreamKind::Subtitle, StreamKind::Attachment, StreamKind::Data] {
            assert_eq!(StreamKind::parse(kind.as_db_str()), kind);
        }
        assert_eq!(StreamKind::parse("weird"), StreamKind::Data);
    }
}
