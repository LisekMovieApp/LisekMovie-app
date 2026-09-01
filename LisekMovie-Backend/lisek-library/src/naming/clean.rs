//! Shared string-cleaning helpers used by both the movie and episode
//! parsers: a "clean string" pass that runs before trying to pull a year
//! or a season/episode marker out of a filename — strip
//! separators down to plain spaces, then strip the trailing "junk" (release
//! group tag, quality/codec tags) that quality-conscious release names pile
//! on after the meaningful part of the name.

use std::sync::LazyLock;

use regex::Regex;

/// Recognized release-quality / source / codec / audio tokens. Not
/// exhaustive — real-world scene release tags run to dozens of regional and
/// scene-specific variants — but it covers the tags that show up in the
/// overwhelming majority of real-world file names, which is what a "strip
/// everything from the first tag onward" heuristic needs to work well.
///
/// Deliberately has no bare single-digit entries (no `"1"`, `"5"`, ...):
/// after `normalize_separators` turns every `.`/`_`/`-` run into a plain
/// space, a channel-layout tag like `DD5.1` ends up as two words, `dd5` and
/// `1`. Treating `"1"` itself as junk would truncate any title that happens
/// to contain a lone digit — `Ocean's Eleven Part 1` becoming `Ocean's
/// Eleven Part` — which is a far more common and worse failure than
/// occasionally leaving a `1` from an audio tag in an already-doomed
/// filename that has no year or S/E marker to split on in the first place.
const JUNK_TOKENS: &[&str] = &[
    "2160p",
    "1080p",
    "720p",
    "480p",
    "576p",
    "4k",
    "8k",
    "uhd",
    "hdr",
    "hdr10",
    "hdr10plus",
    "dolbyvision",
    "sdr",
    "bluray",
    "blu-ray",
    "bdrip",
    "brrip",
    "bdremux",
    "remux",
    "webdl",
    "web-dl",
    "webrip",
    "web",
    "hdtv",
    "pdtv",
    "dsr",
    "dvdrip",
    "dvdscr",
    "dvd",
    "hdcam",
    "hdts",
    "cam",
    "telesync",
    "x264",
    "x265",
    "h264",
    "h265",
    "hevc",
    "avc",
    "xvid",
    "divx",
    "aac",
    "aac2",
    "ac3",
    "eac3",
    "dd5",
    "ddp5",
    "dts",
    "dtshd",
    "atmos",
    "truehd",
    "flac",
    "proper",
    "repack",
    "extended",
    "unrated",
    "uncut",
    "remastered",
    "imax",
    "limited",
    "internal",
    "retail",
    "multi",
    "dubbed",
    "subbed",
    // Polish-release dub/subtitle tags — this project ships a Polish UI and
    // its own real-world test library turned up titles ("Jak rozpętałem
    // II wojnę światową", "Asterix i Obelix...") that TMDB search missed
    // because a tag like "Lektor.PL" sitting *before* the year in the
    // filename (a real, if non-standard, release convention) was leaking
    // into the parsed title instead of being recognized as junk. Bare
    // "pl" is deliberately included despite being only two letters — like
    // the bare "web"/"cam"/"dvd" tokens above, the risk of it colliding
    // with an actual two-letter word in a movie title is far smaller than
    // the cost of leaving a real language tag stuck in every Polish
    // release's parsed title.
    "lektor",
    "lektorpl",
    "napisy",
    "napisypl",
    "dubbingpl",
    "pl",
];

static SEPARATOR_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s._-]+").unwrap());
static TRAILING_JUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\s\-]+$").unwrap());
static BRACKETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\[({][^\])}]*[\])}]").unwrap());
// A bare resolution written as WIDTHxHEIGHT ("1920x1080", "3840x2160")
// rather than the "1080p"/"4K" shorthand already in `JUNK_TOKENS`.
static RESOLUTION_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\d{3,4}x\d{3,4}$").unwrap());

fn is_junk_word(word: &str) -> bool {
    JUNK_TOKENS.contains(&word.to_lowercase().as_str()) || RESOLUTION_TAG.is_match(word)
}

/// Normalizes `.`/`_`/`-` separators to plain spaces and collapses runs of
/// whitespace/separators to a single space, without touching letters or
/// digits themselves. `Movie.Title.Here` and `Movie_Title_Here` and
/// `Movie Title Here` all come out identical, which is what lets a single
/// regex set handle every separator convention scene releases use.
pub fn normalize_separators(input: &str) -> String {
    SEPARATOR_RUN
        .replace_all(input.trim(), " ")
        .trim()
        .to_string()
}

/// Drops any `[...]`, `(...)`, or `{...}` bracketed group — used for a
/// release-group tag like `[YIFY]` or an edition/tag block — from `input`.
/// Kept separate from year extraction, which deliberately looks *inside*
/// parens/brackets for a year before this strips them.
pub fn strip_bracketed_groups(input: &str) -> String {
    BRACKETED.replace_all(input, " ").to_string()
}

/// Trims a cleaned, whitespace-split title down to the words before the
/// first recognized junk token (quality, source, codec, audio, edition
/// marker, ...), so `"Movie Title 1080p BluRay x264-GROUP"` becomes
/// `"Movie Title"`. Case-insensitive; matches whole normalized words only.
pub fn strip_trailing_junk(words: &[&str]) -> String {
    let cut = words
        .iter()
        .position(|w| is_junk_word(w))
        .unwrap_or(words.len());
    let title = words[..cut].join(" ");
    TRAILING_JUNK.replace(title.trim(), "").trim().to_string()
}

/// Final polish on an extracted title: collapses whitespace, and trims
/// stray leading/trailing separator characters (dashes, colons) that are
/// left over once a year or season/episode marker has been cut out of the
/// middle or end of a normalized name.
pub fn tidy_title(raw: &str) -> String {
    let collapsed = normalize_separators(raw);
    collapsed
        .trim_matches(|c: char| c == '-' || c == ':' || c.is_whitespace())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_separators_unifies_dots_underscores_and_spaces() {
        assert_eq!(normalize_separators("Movie.Title_Here"), "Movie Title Here");
        assert_eq!(normalize_separators("Movie   Title"), "Movie Title");
        assert_eq!(normalize_separators("  Movie.Title  "), "Movie Title");
    }

    #[test]
    fn strip_trailing_junk_cuts_at_first_quality_tag() {
        let words: Vec<&str> = "Movie Title 1080p BluRay x264 GROUP".split(' ').collect();
        assert_eq!(strip_trailing_junk(&words), "Movie Title");
    }

    #[test]
    fn strip_trailing_junk_keeps_whole_title_when_no_junk_present() {
        let words: Vec<&str> = "Plain Title".split(' ').collect();
        assert_eq!(strip_trailing_junk(&words), "Plain Title");
    }

    #[test]
    fn strip_trailing_junk_does_not_truncate_a_title_containing_a_lone_digit() {
        let words: Vec<&str> = "Ocean's Eleven Part 1".split(' ').collect();
        assert_eq!(strip_trailing_junk(&words), "Ocean's Eleven Part 1");
    }

    #[test]
    fn strip_trailing_junk_treats_a_bare_resolution_tag_as_junk() {
        let words: Vec<&str> = "Movie Title 1920x1080 x264".split(' ').collect();
        assert_eq!(strip_trailing_junk(&words), "Movie Title");
    }

    #[test]
    fn strip_bracketed_groups_removes_release_group_tag() {
        assert_eq!(
            strip_bracketed_groups("Movie Title [YIFY]").trim(),
            "Movie Title"
        );
    }

    #[test]
    fn tidy_title_trims_stray_separators() {
        assert_eq!(tidy_title(" - Movie Title - "), "Movie Title");
        assert_eq!(tidy_title("Movie Title -"), "Movie Title");
    }
}
