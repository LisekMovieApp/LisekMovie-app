//! HLS playlists and the names of the files a session serves.
//!
//! The video playlist is a complete VOD list written before the first
//! segment exists, so players show the real duration and can seek
//! anywhere; a segment that is not produced yet is made on request (see
//! [`crate::session`]).

use crate::keyframes::SegmentMap;
use crate::subtitles::TrackRef;

/// A file in `/api/playback/{session}/hls/{file}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HlsFile {
    Master,
    Video,
    Init,
    Segment(usize),
    SubtitlePlaylist(TrackRef),
    SubtitleSegment(TrackRef, usize),
}

fn number(value: &str) -> Option<usize> {
    (!value.is_empty() && value.len() <= 6 && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

impl HlsFile {
    /// Accepts only the names this module generates.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "master.m3u8" => return Some(HlsFile::Master),
            "video.m3u8" => return Some(HlsFile::Video),
            "init.mp4" => return Some(HlsFile::Init),
            _ => {}
        }
        if let Some(n) = name.strip_prefix("seg-").and_then(|r| r.strip_suffix(".m4s")) {
            return number(n).map(HlsFile::Segment);
        }
        let rest = name.strip_prefix("subs-")?;
        if let Some(id) = rest.strip_suffix(".m3u8") {
            return TrackRef::parse(id).map(HlsFile::SubtitlePlaylist);
        }
        let (id, n) = rest.strip_suffix(".vtt")?.split_once('-')?;
        Some(HlsFile::SubtitleSegment(TrackRef::parse(id)?, number(n)?))
    }

    pub fn content_type(self) -> &'static str {
        match self {
            HlsFile::Master | HlsFile::Video | HlsFile::SubtitlePlaylist(_) => {
                "application/vnd.apple.mpegurl"
            }
            HlsFile::Init => "video/mp4",
            HlsFile::Segment(_) => "video/iso.segment",
            HlsFile::SubtitleSegment(..) => "text/vtt; charset=utf-8",
        }
    }
}

pub fn segment_name(index: usize) -> String {
    format!("seg-{index}.m4s")
}

/// The full-length VOD media playlist. `query` (`?ticket=...`) is appended
/// to every URI.
pub fn video_playlist(map: &SegmentMap, query: &str) -> String {
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-INDEPENDENT-SEGMENTS\n#EXT-X-MAP:URI=\"init.mp4{query}\"\n",
        map.longest().ceil().max(1.0) as u64
    );
    for index in 0..map.len() {
        out.push_str(&format!("#EXTINF:{:.6},\n{}{query}\n", map.length(index), segment_name(index)));
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out
}

/// A subtitle rendition in the master playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendition {
    pub track: TrackRef,
    pub name: String,
    pub language: Option<String>,
    pub default: bool,
    pub forced: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub bandwidth_bps: u64,
    pub codecs: Option<String>,
    pub width: u32,
    pub height: u32,
}

fn quoted(value: &str) -> String {
    value.replace(['"', '\n', '\r'], "'")
}

pub fn master_playlist(variant: &Variant, renditions: &[Rendition], query: &str) -> String {
    let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-INDEPENDENT-SEGMENTS\n");
    for r in renditions {
        out.push_str(&format!(
            "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"{}\",{}DEFAULT={},AUTOSELECT={},FORCED={},URI=\"subs-{}.m3u8{query}\"\n",
            quoted(&r.name),
            r.language
                .as_deref()
                .map(|l| format!("LANGUAGE=\"{}\",", quoted(l)))
                .unwrap_or_default(),
            if r.default { "YES" } else { "NO" },
            if r.default || r.forced { "YES" } else { "NO" },
            if r.forced { "YES" } else { "NO" },
            r.track.id(),
        ));
    }
    out.push_str(&format!(
        "#EXT-X-STREAM-INF:BANDWIDTH={},RESOLUTION={}x{}",
        variant.bandwidth_bps.max(1),
        variant.width,
        variant.height
    ));
    if let Some(codecs) = &variant.codecs {
        out.push_str(&format!(",CODECS=\"{}\"", quoted(codecs)));
    }
    if !renditions.is_empty() {
        out.push_str(",SUBTITLES=\"subs\"");
    }
    out.push_str(&format!("\nvideo.m3u8{query}\n"));
    out
}

/// Subtitle rendition playlist: one WebVTT segment per video segment for
/// subtitles exported while streaming, or a single segment spanning the
/// whole file for an external file converted at once.
pub fn subtitle_playlist(map: &SegmentMap, track: TrackRef, segmented: bool, query: &str) -> String {
    let id = track.id();
    let target = if segmented { map.longest() } else { map.duration() };
    let mut out = format!(
        "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:{}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n",
        target.ceil().max(1.0) as u64
    );
    if segmented {
        for index in 0..map.len() {
            out.push_str(&format!("#EXTINF:{:.6},\nsubs-{id}-{index}.vtt{query}\n", map.length(index)));
        }
    } else {
        out.push_str(&format!("#EXTINF:{:.6},\nsubs-{id}-0.vtt{query}\n", map.duration()));
    }
    out.push_str("#EXT-X-ENDLIST\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_generated_names() {
        assert_eq!(HlsFile::parse("master.m3u8"), Some(HlsFile::Master));
        assert_eq!(HlsFile::parse("seg-12.m4s"), Some(HlsFile::Segment(12)));
        assert_eq!(
            HlsFile::parse("subs-s4-7.vtt"),
            Some(HlsFile::SubtitleSegment(TrackRef::Embedded(4), 7))
        );
        assert_eq!(HlsFile::parse("subs-x0.m3u8"), Some(HlsFile::SubtitlePlaylist(TrackRef::External(0))));
        for bad in [
            "../init.mp4", "seg-.m4s", "seg-1234567.m4s", "seg-1.ts", "subs-q1.m3u8", "subs-s1-.vtt",
            "index.m3u8", "ffmpeg.log", "seg-1.m4s.part", "subs-s1-2-3.vtt",
        ] {
            assert_eq!(HlsFile::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn playlists_carry_the_ticket_and_real_durations() {
        let map = SegmentMap::fixed(10.0, 4.0);
        let video = video_playlist(&map, "?ticket=abc");
        assert!(video.contains("#EXT-X-TARGETDURATION:4\n"));
        assert!(video.contains("#EXT-X-MAP:URI=\"init.mp4?ticket=abc\"\n"));
        assert!(video.contains("#EXTINF:2.000000,\nseg-2.m4s?ticket=abc\n"));
        assert!(video.ends_with("#EXT-X-ENDLIST\n"));

        let master = master_playlist(
            &Variant { bandwidth_bps: 5_000_000, codecs: Some("avc1.640028,mp4a.40.2".into()), width: 1920, height: 1080 },
            &[Rendition { track: TrackRef::Embedded(4), name: "Polski \"PL\"".into(), language: Some("pol".into()), default: true, forced: false }],
            "?ticket=abc",
        );
        assert!(master.contains("NAME=\"Polski 'PL'\",LANGUAGE=\"pol\",DEFAULT=YES,AUTOSELECT=YES,FORCED=NO,URI=\"subs-s4.m3u8?ticket=abc\""));
        assert!(master.contains("BANDWIDTH=5000000,RESOLUTION=1920x1080,CODECS=\"avc1.640028,mp4a.40.2\",SUBTITLES=\"subs\"\nvideo.m3u8?ticket=abc\n"));

        let whole = subtitle_playlist(&map, TrackRef::External(0), false, "?t=1");
        assert!(whole.contains("#EXTINF:10.000000,\nsubs-x0-0.vtt?t=1\n"));
        assert!(subtitle_playlist(&map, TrackRef::Embedded(4), true, "").contains("subs-s4-2.vtt\n"));
    }
}
