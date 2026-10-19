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
    pub streams: Vec<MediaStream>,
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
        }
    }

    #[test]
    fn primary_video_skips_embedded_cover_art() {
        let probe = MediaProbe {
            container: None,
            duration_seconds: None,
            bitrate_kbps: None,
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
            streams: vec![stream(1, StreamKind::Audio, "aac"), second],
        };
        assert_eq!(probe.primary_audio().unwrap().index, 2);
    }

    #[test]
    fn stream_kind_round_trips_and_defaults_to_data() {
        for kind in [StreamKind::Video, StreamKind::Audio, StreamKind::Subtitle, StreamKind::Attachment, StreamKind::Data] {
            assert_eq!(StreamKind::parse(kind.as_db_str()), kind);
        }
        assert_eq!(StreamKind::parse("weird"), StreamKind::Data);
    }
}
