//! Subtitle tracks: embedded streams (ids `s<stream index>`) and text
//! files next to the video (ids `x<n>`), their delivery as WebVTT, and the
//! cue bookkeeping behind segmented HLS subtitle renditions.
//!
//! Text subtitles reach the player as WebVTT: a `<track>` file for direct
//! play, an HLS rendition otherwise. Picture-based ones (PGS, VobSub, DVB)
//! cannot be converted to text and are burnt into the video.

use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleKind {
    Text,
    Image,
}

/// `None` for formats FFmpeg cannot turn into either (e.g. teletext).
pub fn kind_of_codec(codec: &str) -> Option<SubtitleKind> {
    match codec {
        "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "mov_text" | "text" | "microdvd" | "mpl2"
        | "subviewer" | "subviewer1" | "sami" | "realtext" | "jacosub" | "pjs" | "vplayer"
        | "stl" => Some(SubtitleKind::Text),
        "hdmv_pgs_subtitle" | "dvd_subtitle" | "dvb_subtitle" | "xsub" => Some(SubtitleKind::Image),
        _ => None,
    }
}

/// A subtitle file beside the video, e.g. `Film (2010).pol.forced.srt`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalSubtitle {
    pub path: PathBuf,
    /// Lower-case extension: `srt`, `ass`, `ssa`, `vtt`, `txt`, `sub`,
    /// `sup` (PGS) or `idx` (VobSub).
    pub format: String,
    /// ISO 639-2 code when the file name carries a known language.
    pub language: Option<String>,
    pub title: Option<String>,
    pub forced: bool,
    pub default: bool,
    pub kind: SubtitleKind,
}

/// Which track a client picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackRef {
    /// ffprobe stream index of an embedded subtitle stream.
    Embedded(i32),
    /// Position in the external subtitle list.
    External(usize),
}

impl TrackRef {
    pub fn parse(id: &str) -> Option<Self> {
        let (kind, number) = id.split_at_checked(1)?;
        if number.is_empty() || number.len() > 4 || !number.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        match kind {
            "s" => number.parse().ok().map(TrackRef::Embedded),
            "x" => number.parse().ok().map(TrackRef::External),
            _ => None,
        }
    }

    pub fn id(self) -> String {
        match self {
            TrackRef::Embedded(index) => format!("s{index}"),
            TrackRef::External(n) => format!("x{n}"),
        }
    }
}

const EXTENSIONS: &[(&str, SubtitleKind)] = &[
    ("srt", SubtitleKind::Text),
    ("ass", SubtitleKind::Text),
    ("ssa", SubtitleKind::Text),
    ("vtt", SubtitleKind::Text),
    ("txt", SubtitleKind::Text),
    ("sub", SubtitleKind::Text),
    ("sup", SubtitleKind::Image),
    ("idx", SubtitleKind::Image),
];

/// Language names and codes seen in subtitle file names, as ISO 639-2.
const LANGUAGES: &[(&[&str], &str)] = &[
    (&["pl", "pol", "polish", "polski"], "pol"),
    (&["en", "eng", "english"], "eng"),
    (&["de", "ger", "deu", "german", "deutsch"], "ger"),
    (&["fr", "fre", "fra", "french"], "fre"),
    (&["es", "spa", "spanish"], "spa"),
    (&["it", "ita", "italian"], "ita"),
    (&["ru", "rus", "russian"], "rus"),
    (&["uk", "ukr", "ukrainian"], "ukr"),
    (&["cs", "cze", "ces", "czech"], "cze"),
    (&["sk", "slo", "slk", "slovak"], "slo"),
    (&["ja", "jpn", "japanese"], "jpn"),
    (&["ko", "kor", "korean"], "kor"),
    (&["zh", "chi", "zho", "chinese"], "chi"),
];

/// Subtitle files belonging to `video`: same directory, file name
/// starting with the video's stem. Sorted by name, so `x<n>` ids are
/// stable while the directory does not change. Blocking (reads the
/// directory); run it on a blocking thread.
pub fn find_external(video: &Path) -> Vec<ExternalSubtitle> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str())) else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|(name, path)| describe_external(stem, &name, path))
        .collect()
}

/// Parses `<stem>[.<tag>...].<ext>` into an [`ExternalSubtitle`]. Pure,
/// for tests.
pub fn describe_external(stem: &str, file_name: &str, path: PathBuf) -> Option<ExternalSubtitle> {
    let rest = file_name.strip_prefix(stem)?;
    let (tags, ext) = rest.rsplit_once('.')?;
    if !(tags.is_empty() || tags.starts_with('.')) {
        return None; // "Film 2.srt" belongs to "Film 2", not "Film"
    }
    let format = ext.to_ascii_lowercase();
    let kind = EXTENSIONS.iter().find(|(e, _)| *e == format)?.1;
    let mut language = None;
    let mut forced = false;
    let mut default = false;
    let mut title_parts = vec![];
    for tag in tags.split('.').filter(|t| !t.is_empty()) {
        let lower = tag.to_ascii_lowercase();
        match lower.as_str() {
            "forced" | "foreign" => forced = true,
            "default" => default = true,
            "sdh" | "cc" | "hi" => title_parts.push("SDH".to_owned()),
            _ => match LANGUAGES.iter().find(|(names, _)| names.contains(&lower.as_str())) {
                Some((_, code)) if language.is_none() => language = Some((*code).to_owned()),
                _ => title_parts.push(tag.to_owned()),
            },
        }
    }
    Some(ExternalSubtitle {
        path,
        format,
        language,
        title: (!title_parts.is_empty()).then(|| title_parts.join(" ")),
        forced,
        default,
        kind,
    })
}

/// First non-empty line looks like MicroDVD (`{123}{456}text`), which
/// FFmpeg times by frame and therefore needs the video's frame rate.
pub fn is_microdvd(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
    let line = text
        .trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut parts = line.splitn(3, '}');
    let first = parts.next().unwrap_or_default();
    let second = parts.next().unwrap_or_default();
    first.starts_with('{')
        && first[1..].bytes().all(|b| b.is_ascii_digit())
        && first.len() > 1
        && second.starts_with('{')
        && second[1..].bytes().all(|b| b.is_ascii_digit())
}

/// Text that is not UTF-8 (and has no UTF-16 byte order mark) needs an
/// explicit `-sub_charenc`.
pub fn needs_charset(bytes: &[u8]) -> bool {
    let utf16 = bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]);
    !utf16 && std::str::from_utf8(bytes).is_err()
}

/// ffmpeg arguments converting a text subtitle file to WebVTT on stdout.
pub fn external_to_vtt_args(
    path: &Path,
    charset: Option<&str>,
    microdvd_fps: Option<f64>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-hide_banner", "-v", "error", "-nostdin"]
        .iter()
        .map(OsString::from)
        .collect();
    if let Some(charset) = charset {
        args.extend(["-sub_charenc".into(), charset.into()]);
    }
    if let Some(fps) = microdvd_fps {
        args.extend(["-subfps".into(), format!("{fps:.3}").into()]);
    }
    args.push("-i".into());
    args.push(path.into());
    args.extend(
        ["-map", "0:s:0", "-c:s", "webvtt", "-f", "webvtt", "pipe:1"]
            .iter()
            .map(OsString::from),
    );
    args
}

/// One WebVTT cue, times in seconds on the playlist timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    /// Cue settings after the timing (`line:85%` ...), may be empty.
    pub settings: String,
    pub text: String,
}

fn parse_timestamp(value: &str) -> Option<f64> {
    // hh:mm:ss.ttt or mm:ss.ttt
    let (main, millis) = value.trim().split_once('.')?;
    let parts: Vec<&str> = main.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (h.parse::<u64>().ok()?, m.parse::<u64>().ok()?, s.parse::<u64>().ok()?),
        [m, s] => (0, m.parse::<u64>().ok()?, s.parse::<u64>().ok()?),
        _ => return None,
    };
    let ms: u64 = millis.get(..3)?.parse().ok()?;
    Some((h * 3600 + m * 60 + s) as f64 + ms as f64 / 1000.0)
}

pub fn format_timestamp(seconds: f64) -> String {
    let total_ms = (seconds.max(0.0) * 1000.0).round() as u64;
    let (h, rest) = (total_ms / 3_600_000, total_ms % 3_600_000);
    let (m, rest) = (rest / 60_000, rest % 60_000);
    format!("{h:02}:{m:02}:{:02}.{:03}", rest / 1000, rest % 1000)
}

/// Parses the cues in `text` (WebVTT as written by FFmpeg). Returns the
/// cues and how many bytes were consumed, so a growing file can be read
/// incrementally. FFmpeg writes each cue in one flushed write ending in a
/// newline and puts the blank separator line before the *next* cue, so
/// everything up to the last newline is complete; the rest waits for the
/// next read (or `at_eof`). `offset` is subtracted from every timestamp.
pub fn parse_vtt(text: &str, offset: f64, at_eof: bool) -> (Vec<Cue>, usize) {
    let complete = if at_eof {
        text.len()
    } else {
        text.rfind('\n').map_or(0, |i| i + 1)
    };
    let mut cues = vec![];
    let mut block: Vec<&str> = vec![];
    let mut flush = |block: &mut Vec<&str>| {
        if let Some(cue) = parse_cue(block, offset) {
            cues.push(cue);
        }
        block.clear();
    };
    for line in text[..complete].lines().map(|l| l.trim_end_matches('\r')) {
        if line.trim().is_empty() {
            flush(&mut block);
        } else {
            block.push(line);
        }
    }
    flush(&mut block);
    (cues, complete)
}

/// One block: optional identifier, timing line, text lines. Header,
/// NOTE and STYLE blocks yield `None`.
fn parse_cue(block: &[&str], offset: f64) -> Option<Cue> {
    let timing = block.iter().position(|l| l.contains("-->"))?;
    if timing > 1 || block[0].starts_with("WEBVTT") || block[0].starts_with("NOTE") {
        return None;
    }
    let (start, rest) = block[timing].split_once("-->")?;
    let rest = rest.trim();
    let (end, settings) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let (start, end) = (parse_timestamp(start)?, parse_timestamp(end)?);
    let text = &block[timing + 1..];
    if text.is_empty() {
        return None;
    }
    Some(Cue {
        start: start - offset,
        end: end - offset,
        settings: settings.trim().to_owned(),
        text: text.join("\n"),
    })
}

/// Cues of one subtitle stream collected from successive FFmpeg runs.
/// Runs overlap after a seek, so cues are keyed by timing and text.
#[derive(Debug, Default)]
pub struct CueStore {
    cues: BTreeMap<(i64, i64, String), Cue>,
}

impl CueStore {
    pub fn insert(&mut self, cue: Cue) {
        let key = (
            (cue.start * 1000.0).round() as i64,
            (cue.end * 1000.0).round() as i64,
            cue.text.clone(),
        );
        self.cues.entry(key).or_insert(cue);
    }

    pub fn len(&self) -> usize {
        self.cues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// Cues showing at any moment of `[start, end)`.
    pub fn overlapping(&self, start: f64, end: f64) -> impl Iterator<Item = &Cue> {
        self.cues
            .values()
            .filter(move |c| c.start < end && c.end > start)
    }
}

/// WebVTT document for one HLS subtitle segment. `mpegts` maps the cue
/// timeline (LOCAL 0) to the media timestamps in the video segments, as
/// HLS requires for WebVTT renditions.
pub fn vtt_document<'a>(cues: impl Iterator<Item = &'a Cue>, mpegts: Option<u64>) -> String {
    let mut out = String::from("WEBVTT\n");
    if let Some(mpegts) = mpegts {
        out.push_str(&format!("X-TIMESTAMP-MAP=MPEGTS:{mpegts},LOCAL:00:00:00.000\n"));
    }
    out.push('\n');
    for cue in cues {
        out.push_str(&format_timestamp(cue.start));
        out.push_str(" --> ");
        out.push_str(&format_timestamp(cue.end));
        if !cue.settings.is_empty() {
            out.push(' ');
            out.push_str(&cue.settings);
        }
        out.push('\n');
        out.push_str(&cue.text);
        out.push_str("\n\n");
    }
    out
}

/// Escapes a value for an FFmpeg filter option inside a filter graph
/// (option level, then graph level), e.g. a file name for `subtitles=`.
pub fn escape_filter_value(value: &str) -> String {
    // Option level: \ ' : are special.
    let mut level1 = String::with_capacity(value.len() + 8);
    for c in value.chars() {
        if matches!(c, '\\' | '\'' | ':') {
            level1.push('\\');
        }
        level1.push(c);
    }
    // Graph level: \ ' [ ] , ; are special.
    let mut level2 = String::with_capacity(level1.len() + 8);
    for c in level1.chars() {
        if matches!(c, '\\' | '\'' | '[' | ']' | ',' | ';') {
            level2.push('\\');
        }
        level2.push(c);
    }
    level2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_ids_round_trip_and_reject_garbage() {
        assert_eq!(TrackRef::parse("s4"), Some(TrackRef::Embedded(4)));
        assert_eq!(TrackRef::parse("x0"), Some(TrackRef::External(0)));
        for bad in ["", "s", "y1", "s-1", "s12345", "x1/../", "s1.vtt"] {
            assert_eq!(TrackRef::parse(bad), None, "{bad}");
        }
        assert_eq!(TrackRef::External(3).id(), "x3");
    }

    #[test]
    fn external_names_give_language_flags_and_kind() {
        let d = |name: &str| describe_external("Film (2010)", name, PathBuf::from(name));
        let pl = d("Film (2010).pol.forced.srt").unwrap();
        assert_eq!(pl.language.as_deref(), Some("pol"));
        assert!(pl.forced && pl.kind == SubtitleKind::Text && pl.format == "srt");
        let en = d("Film (2010).EN.SDH.ass").unwrap();
        assert_eq!((en.language.as_deref(), en.title.as_deref()), (Some("eng"), Some("SDH")));
        assert_eq!(d("Film (2010).sup").unwrap().kind, SubtitleKind::Image);
        assert_eq!(d("Film (2010).Napisy24.txt").unwrap().title.as_deref(), Some("Napisy24"));
        assert!(d("Film (2010).nfo").is_none());
        assert!(d("Film (2010) 2.srt").is_none(), "another video's subtitles");
    }

    #[test]
    fn detects_microdvd_and_legacy_charsets() {
        assert!(is_microdvd(b"{1}{1}23.976\n{100}{200}Tekst"));
        assert!(is_microdvd(b"\xef\xbb\xbf{0}{25}Tekst"));
        assert!(!is_microdvd(b"1\n00:00:01,000 --> 00:00:02,000\nx"));
        assert!(needs_charset(b"Za\xbf\xf3\xb3\xe6")); // "Zażółć" in CP1250
        assert!(!needs_charset("Zażółć".as_bytes()));
        assert!(!needs_charset(&[0xFF, 0xFE, b'a', 0]));
    }

    #[test]
    fn a_cue_is_complete_once_its_line_ends() {
        // As FFmpeg writes it: no blank line after the last cue yet.
        let text = "WEBVTT\n\n00:01.000 --> 00:03.000\nPierwszy napis\n";
        let (cues, used) = parse_vtt(text, 0.0, false);
        assert_eq!(cues.len(), 1);
        assert_eq!(used, text.len());
        let more = "\n00:21.500 --> 00:23.000\nDrugi\n";
        let (cues, _) = parse_vtt(more, 0.0, false);
        assert_eq!(cues[0].text, "Drugi");
        let numbered = "1\n00:00:01.000 --> 00:00:02.000\nz numerem\n\nNOTE x\n\n";
        let (cues, used) = parse_vtt(numbered, 0.0, false);
        assert_eq!((cues.len(), cues[0].text.as_str(), used), (1, "z numerem", numbered.len()));
    }

    #[test]
    fn parses_ffmpeg_vtt_incrementally() {
        let text = "WEBVTT\n\n00:21.500 --> 00:23.000\nZażółć\ngęślą\n\n1:00:40.000 --> 1:00:42.000 line:90%\nTrzeci\n\n00:50.000 --> 00:5";
        let (cues, used) = parse_vtt(text, 1.0, false);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], Cue { start: 20.5, end: 22.0, settings: String::new(), text: "Zażółć\ngęślą".into() });
        assert_eq!(cues[1].start, 3639.0);
        assert_eq!(cues[1].settings, "line:90%");
        assert_eq!(&text[used..], "00:50.000 --> 00:5", "the unfinished cue waits");
    }

    #[test]
    fn cue_store_deduplicates_and_cuts_segments() {
        let mut store = CueStore::default();
        let cue = |s: f64, e: f64, t: &str| Cue { start: s, end: e, settings: String::new(), text: t.into() };
        store.insert(cue(1.0, 3.0, "a"));
        store.insert(cue(1.0, 3.0, "a"));
        store.insert(cue(3.5, 9.0, "b"));
        assert_eq!(store.len(), 2);
        let seg: Vec<_> = store.overlapping(4.0, 8.0).map(|c| c.text.as_str()).collect();
        assert_eq!(seg, ["b"]);
        let doc = vtt_document(store.overlapping(0.0, 4.0), Some(900000));
        assert!(doc.starts_with("WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n"));
        assert!(doc.contains("00:00:01.000 --> 00:00:03.000\na\n\n"));
        assert_eq!(format_timestamp(3725.5), "01:02:05.500");
    }

    #[test]
    fn filter_values_are_escaped_twice() {
        assert_eq!(escape_filter_value("/m/a b.mkv"), "/m/a b.mkv");
        assert_eq!(escape_filter_value("C:\\Film's [x], y;.mkv"), "C\\\\:\\\\\\\\Film\\\\\\'s \\[x\\]\\, y\\;.mkv");
    }
}
