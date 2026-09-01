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
///
/// Deliberately does NOT include the language/dub/subtitle tags
/// (`"lektor"`, `"pl"`, ...) that used to live in this same list — see
/// [`LANGUAGE_TAG_TOKENS`] for why those need different handling, and
/// [`is_trailing_junk_word`]/[`strip_trailing_junk`] for how the two lists
/// are actually combined.
const TRAILING_JUNK_TOKENS: &[&str] = &[
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
];

/// Polish-release dub/subtitle tags — split out from
/// [`TRAILING_JUNK_TOKENS`] (Faza 5 completion round) once a *second* real
/// title in this project's own test library showed why the two lists need
/// different handling. The first fix (round 2) added these to the trailing
/// list to handle `Asterix.i.Obelix.Lektor.PL.2012...` — a language tag
/// sitting directly before the year, with nothing real after it, where
/// "cut at first hit" gives the right answer either way. This round's
/// title, a *subtitled* entry in the same franchise —
/// `Asterix.i.Obelix.Lektor.PL.W.Sluzbie.Jej.Krolewskiej.Mosci.2012...` —
/// exposed the gap: the tag here sits BETWEEN the base title and a real
/// subtitle, so "cut at first hit" truncated the parsed title down to just
/// "Asterix i Obelix", silently dropping "W Służbie Jej Królewskiej
/// Mości" and sending the wrong (if plausible-looking) query to every
/// metadata provider — no error, no empty result, just a match against
/// the wrong film in the franchise, or no match at all if that shorter
/// title isn't in the provider's catalog either.
///
/// Unlike [`TRAILING_JUNK_TOKENS`] (which real-world scene/web-release
/// convention reliably clusters into one unbroken run at the very end of a
/// filename, right before an unrecognized release-group name), a
/// language/dub tag has no such guarantee — it can trail the base title,
/// trail the full title including subtitle, or even lead the whole
/// release name. So these are handled differently: filtered OUT of the
/// word list wherever they appear ([`strip_language_tags`]) instead of
/// marking a truncation point, which correctly recovers a subtitle that
/// comes after the tag while still removing the tag itself. Bare "pl" is
/// deliberately included despite being only two letters — like the bare
/// "web"/"cam"/"dvd" tokens in [`TRAILING_JUNK_TOKENS`], the risk of it
/// colliding with an actual two-letter word in a movie title is far
/// smaller than the cost of leaving a real language tag stuck in every
/// Polish release's parsed title.
const LANGUAGE_TAG_TOKENS: &[&str] = &[
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
// rather than the "1080p"/"4K" shorthand already in `TRAILING_JUNK_TOKENS`.
static RESOLUTION_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\d{3,4}x\d{3,4}$").unwrap());

fn is_trailing_junk_word(word: &str) -> bool {
    TRAILING_JUNK_TOKENS.contains(&word.to_lowercase().as_str()) || RESOLUTION_TAG.is_match(word)
}

fn is_language_tag(word: &str) -> bool {
    LANGUAGE_TAG_TOKENS.contains(&word.to_lowercase().as_str())
}

/// Removes every recognized language/dub/subtitle tag from `words`,
/// wherever it appears — see [`LANGUAGE_TAG_TOKENS`] for why these can't
/// share [`strip_trailing_junk`]'s "cut at the first hit" handling of
/// [`TRAILING_JUNK_TOKENS`].
fn strip_language_tags<'a>(words: &[&'a str]) -> Vec<&'a str> {
    words
        .iter()
        .copied()
        .filter(|w| !is_language_tag(w))
        .collect()
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

/// First removes every recognized language/dub/subtitle tag
/// ([`LANGUAGE_TAG_TOKENS`]) from `words`, wherever it appears, then trims
/// what's left down to the words before the first recognized trailing junk
/// token (quality, source, codec, audio, edition marker, ...) — so
/// `"Movie Title 1080p BluRay x264-GROUP"` becomes `"Movie Title"`, and
/// `"Asterix i Obelix Lektor PL W Sluzbie Jej Krolewskiej Mosci"` becomes
/// `"Asterix i Obelix W Sluzbie Jej Krolewskiej Mosci"` rather than being
/// truncated at "Lektor". Case-insensitive; matches whole normalized words
/// only.
pub fn strip_trailing_junk(words: &[&str]) -> String {
    let words = strip_language_tags(words);
    let cut = words
        .iter()
        .position(|w| is_trailing_junk_word(w))
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
    fn strip_trailing_junk_removes_a_mid_title_language_tag_and_keeps_the_subtitle_after_it() {
        // The Faza 5 completion regression: a language tag sitting BETWEEN
        // a base title and a real subtitle used to be treated the same as
        // a trailing quality tag, truncating everything after it — see
        // `LANGUAGE_TAG_TOKENS`'s own doc comment for the real title that
        // exposed this.
        let words: Vec<&str> = "Asterix i Obelix Lektor PL W Sluzbie Jej Krolewskiej Mosci"
            .split(' ')
            .collect();
        assert_eq!(
            strip_trailing_junk(&words),
            "Asterix i Obelix W Sluzbie Jej Krolewskiej Mosci"
        );
    }

    #[test]
    fn strip_trailing_junk_still_cuts_a_language_tag_immediately_before_the_end() {
        // The original (round 2) case: nothing real follows the tag, so
        // filtering it out vs. truncating at it give the same answer.
        let words: Vec<&str> = "Asterix i Obelix Lektor PL".split(' ').collect();
        assert_eq!(strip_trailing_junk(&words), "Asterix i Obelix");
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
