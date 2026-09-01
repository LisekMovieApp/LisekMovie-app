//! Movie filename parsing: pulling a title and an optional release year out
//! of a video file's base name.
//!
//! Follows a common priority order for extracting a release year from
//! movie filenames (see the Faza 5 research notes): a year written in parentheses or
//! square brackets — `Movie Title (2020)` — is unambiguous and always wins.
//! Failing that, a bare four-digit year token is used, provided it sits
//! between real separators on both sides (so `1920x1080`, a resolution, can
//! never be misread as the year 1920 — the `x` right after the digits is
//! not a separator, so the bare-year regex simply never matches it). When
//! more than one bare year token is present, the *last* one is taken, since
//! the dominant `Title.Year.Quality.Group` convention always puts the real
//! year before the quality/codec tags, and a quality tag never looks like a
//! valid year itself.
//!
//! One known, deliberately unhandled edge case: a movie whose title itself
//! is a bare number that also happens to be a plausible year — `1917`,
//! `2012` — with no parenthesized year and no separate release year in the
//! name at all. That's genuinely ambiguous from the filename alone (a known
//! unsolved problem for filename-only parsing in general — see the research
//! notes on year-in-title ambiguity); the covering test below documents the resulting behavior
//! rather than asserting a "correct" answer that doesn't exist without
//! outside information.

use std::sync::LazyLock;

use regex::Regex;

use super::clean::{normalize_separators, strip_bracketed_groups, strip_trailing_junk, tidy_title};

/// A year parsed out of a movie's filename or containing folder name.
/// Bounded to a plausible release-year range so a stray 4-digit number
/// (a resolution, a bitrate, part of a release-group name) can't slip
/// through even if it happens to sit between separators.
const MIN_YEAR: i32 = 1888; // first known public film screening
const MAX_YEAR_SLACK: i32 = 2; // allow up to 2 years out for pre-release/scanning-lag dates

static PAREN_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\((\d{4})\)").unwrap());
static BRACKET_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[(\d{4})\]").unwrap());
// Bare year: must be flanked by start/end-of-string or whitespace before
// it, with a word boundary after it, once separators have already been
// normalized to spaces — that's what rules out "1920x1080" (no boundary
// between "1920" and the "x" of "x1080"). The trailing check is `\b`
// (zero-width) rather than a consuming `(?:\s|$)`: the `regex` crate has no
// lookahead, so a trailing group that *consumes* the separator would eat
// the very space the *next* match needs to see before it, silently
// dropping every year after the first one when a name contains more than
// one bare year token — `\b` gets the same guarantee without consuming
// anything, since the character after it is what the next search position
// starts from.
static BARE_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|\s)(\d{4})\b").unwrap());

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMovie {
    pub title: String,
    pub year: Option<i32>,
}

fn plausible_year(year: i32, now_year: i32) -> bool {
    year >= MIN_YEAR && year <= now_year + MAX_YEAR_SLACK
}

/// Cleans the text found *before* a detected year for use as a movie title:
/// normalizes separators, cuts at the first recognized junk token (quality,
/// source, codec, audio, edition, or — since Faza 5 round 2 — dub/subtitle
/// language tags), then tidies any stray leading/trailing separators left
/// over. Used by every year-found branch below instead of a bare
/// `tidy_title` call.
///
/// This matters because a release tag doesn't always trail the whole
/// filename — this project's own test library turned up real (if
/// non-standard) releases where a Polish dub tag sits *before* the year,
/// e.g. `Asterix.i.Obelix.Lektor.PL.2012.1080p`. Adding "lektor"/"pl" to
/// `JUNK_TOKENS` alone doesn't fix that case: the year-found branches used
/// to call `tidy_title`, which only normalizes and trims, never recognizes
/// or strips junk words. Only the no-year fallback branch was already
/// calling `strip_trailing_junk`. This helper brings that same junk-cutting
/// behavior to the year-found branches too, so a pre-year language tag gets
/// stripped exactly like a post-year quality tag already was.
fn tidy_and_strip_junk(raw: &str) -> String {
    let normalized = normalize_separators(raw);
    let words: Vec<&str> = normalized.split(' ').filter(|w| !w.is_empty()).collect();
    tidy_title(&strip_trailing_junk(&words))
}

/// Parses a movie's title and release year out of `stem` (the filename
/// without its extension). `now_year` bounds how far in the future a
/// "year" token is still accepted as one — pass the current calendar year;
/// tests pin it so results don't shift as time passes.
pub fn parse_movie_name(stem: &str, now_year: i32) -> ParsedMovie {
    // Search the raw stem first: parens/brackets matter for this pass.
    let normalized = normalize_separators(&strip_bracketed_groups(stem));

    for pattern in [&*PAREN_YEAR, &*BRACKET_YEAR] {
        let plausible = pattern.captures(stem).and_then(|caps| {
            let year = caps[1]
                .parse::<i32>()
                .ok()
                .filter(|&y| plausible_year(y, now_year))?;
            let before = &stem[..caps.get(0).unwrap().start()];
            Some(ParsedMovie {
                title: tidy_and_strip_junk(before),
                year: Some(year),
            })
        });
        if let Some(result) = plausible {
            return result;
        }
    }

    // No parenthesized/bracketed year: fall back to the last plausible bare
    // 4-digit token in the separator-normalized (and bracket-stripped —
    // release-group tags like "[1998]" as a group *name* rather than a
    // year would otherwise confuse this pass) string.
    let mut last_match: Option<(usize, usize, i32)> = None;
    for caps in BARE_YEAR.captures_iter(&normalized) {
        let m = caps.get(1).unwrap();
        let year = m
            .as_str()
            .parse::<i32>()
            .ok()
            .filter(|&y| plausible_year(y, now_year));
        if let Some(year) = year {
            last_match = Some((m.start(), m.end(), year));
        }
    }

    if let Some((start, _end, year)) = last_match {
        let title = tidy_and_strip_junk(&normalized[..start]);
        return ParsedMovie {
            title,
            year: Some(year),
        };
    }

    // No year found at all: the whole (cleaned) name is the title, minus
    // any trailing quality/codec junk that survived without a year to
    // anchor the split.
    let words: Vec<&str> = normalized.split(' ').filter(|w| !w.is_empty()).collect();
    ParsedMovie {
        title: strip_trailing_junk(&words),
        year: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YEAR: i32 = 2026;

    #[test]
    fn parenthesized_year_is_authoritative() {
        let result = parse_movie_name("Movie Title (2020)", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Movie Title".into(),
                year: Some(2020)
            }
        );
    }

    #[test]
    fn bracketed_year_is_recognized() {
        let result = parse_movie_name("Movie Title [2020]", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Movie Title".into(),
                year: Some(2020)
            }
        );
    }

    #[test]
    fn dotted_release_convention_with_quality_and_group_tags() {
        let result = parse_movie_name("Movie.Title.2020.1080p.BluRay.x264-GROUP", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Movie Title".into(),
                year: Some(2020)
            }
        );
    }

    #[test]
    fn underscored_release_convention() {
        let result = parse_movie_name("Movie_Title_2020_720p", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Movie Title".into(),
                year: Some(2020)
            }
        );
    }

    #[test]
    fn plain_spaced_name_with_trailing_year() {
        let result = parse_movie_name("Movie Title 2020", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Movie Title".into(),
                year: Some(2020)
            }
        );
    }

    #[test]
    fn resolution_tag_is_never_mistaken_for_a_year() {
        // "1920x1080" must not be read as the year 1920 — there's no
        // separator between the digits and the "x".
        let result = parse_movie_name("Movie Title 1920x1080 x264", YEAR);
        assert_eq!(result.year, None);
        assert_eq!(result.title, "Movie Title");
    }

    #[test]
    fn no_year_present_falls_back_to_whole_cleaned_title() {
        let result = parse_movie_name("Some Documentary Without A Year", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Some Documentary Without A Year".into(),
                year: None
            }
        );
    }

    #[test]
    fn multiple_bare_years_picks_the_last_plausible_one() {
        // A (contrived but real-world-possible) release note mentioning an
        // earlier year in the title, with the actual release year later.
        let result = parse_movie_name("Best Of 1999 2020 WEBRip", YEAR);
        assert_eq!(result.year, Some(2020));
    }

    #[test]
    fn year_in_title_ambiguity_is_a_known_unsolved_edge_case() {
        // "1917" the movie: with no parenthesized year and no separate
        // release year, the bare-number title is read as the year instead
        // of the title, leaving an empty title. This mirrors a fundamental
        // ambiguity inherent to filename-only parsing; a real library
        // would resolve it via a parenthesized year
        // in the filename ("1917 (2019)") or a provider search on the
        // parent folder name, neither of which this unit covers.
        let result = parse_movie_name("1917", YEAR);
        assert_eq!(result.year, Some(1917));
        assert_eq!(result.title, "");
    }

    #[test]
    fn implausible_future_year_is_not_treated_as_a_year() {
        let result = parse_movie_name("Movie Title 2099", YEAR);
        assert_eq!(result.year, None);
    }

    #[test]
    fn release_group_bracket_tag_does_not_leak_into_title() {
        let result = parse_movie_name("Movie Title (2020) [YIFY]", YEAR);
        assert_eq!(result.title, "Movie Title");
        assert_eq!(result.year, Some(2020));
    }

    #[test]
    fn junk_tag_before_a_bare_year_is_stripped_from_the_title() {
        // Real-world case from the project's own test library: a Polish dub
        // tag placed *before* the year rather than after it. Adding
        // "lektor"/"pl" to JUNK_TOKENS alone can't fix this — it also needs
        // the year-found branch to actually look for junk in the pre-year
        // text instead of only tidying it.
        let result = parse_movie_name("Asterix.i.Obelix.Lektor.PL.2012.1080p", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Asterix i Obelix".into(),
                year: Some(2012)
            }
        );
    }

    #[test]
    fn junk_tag_before_a_parenthesized_year_is_stripped_from_the_title() {
        let result = parse_movie_name("Jak rozpetalem II wojne swiatowa Lektor PL (1969)", YEAR);
        assert_eq!(
            result,
            ParsedMovie {
                title: "Jak rozpetalem II wojne swiatowa".into(),
                year: Some(1969)
            }
        );
    }

    #[test]
    fn subtitled_title_with_a_mid_name_language_tag_keeps_its_full_subtitle() {
        // End-to-end regression for the exact bug reported against this
        // franchise's *subtitled* entry: a Polish dub tag sitting between
        // the base title and a real subtitle used to truncate the parsed
        // title down to just "Asterix i Obelix", silently dropping "W
        // Sluzbie Jej Krolewskiej Mosci" — see `LANGUAGE_TAG_TOKENS` and
        // `strip_trailing_junk` in `naming::clean` for the underlying fix.
        // This test exercises the whole `parse_movie_name` pipeline (not
        // just the `clean` helper in isolation) against a realistic
        // dotted release filename for that title.
        let result = parse_movie_name(
            "Asterix.i.Obelix.Lektor.PL.W.Sluzbie.Jej.Krolewskiej.Mosci.2012.1080p",
            YEAR,
        );
        assert_eq!(
            result,
            ParsedMovie {
                title: "Asterix i Obelix W Sluzbie Jej Krolewskiej Mosci".into(),
                year: Some(2012)
            }
        );
    }
}
