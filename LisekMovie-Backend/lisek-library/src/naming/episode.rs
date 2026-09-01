//! TV episode filename parsing: pulling a series title, season number, and
//! episode number (or a multi-episode range) out of a video file, using
//! both the filename itself and its containing folder names.
//!
//! Priority follows the common season/episode-marker precedence used by
//! self-hosted media servers (see the Faza 5 research notes): an explicit
//! season/episode marker *in the
//! filename* always wins outright — `Show Name S01E02.mkv`, `Show Name
//! 1x02.mkv`, `Show Name Season 1 Episode 2.mkv`. Only when the filename
//! carries no such marker do folder names get consulted at all, and then
//! only to fill in whatever the filename couldn't supply: a season number
//! from a "Season 01" folder, paired with a bare episode marker in the
//! filename (`Episode Title - 02.mkv`), with the series title itself
//! falling back to the show's root folder name in that case, since a
//! per-season folder layout typically doesn't repeat the series name in
//! every episode's filename.
//!
//! Deliberately out of scope for this pass (documented, not silently
//! wrong): anime absolute episode numbering with no season marker at all,
//! and multi-version/"edition" folders — both called out as known
//! complexity in the Faza 5 research notes; a file that doesn't match any
//! pattern here simply comes back `None` and is left for manual
//! classification rather than guessed at.

use std::sync::LazyLock;

use regex::Regex;

use super::clean::{normalize_separators, tidy_title};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEpisode {
    pub series_title: String,
    pub season: i32,
    pub episode: i32,
    /// `Some(n)` for a multi-episode file spanning `episode..=n`
    /// (`S01E02E03` or `S01E02-E03`), `None` for a single episode.
    pub end_episode: Option<i32>,
}

/// The file's own name plus whatever containing folder names are known, so
/// the parser can fall back to them when the filename alone is ambiguous.
/// `season_dir_name` is the immediate parent folder (e.g. `"Season 01"`);
/// `series_dir_name` is the show's root folder, one level further up (or
/// the same as `season_dir_name` for a flat, non-season-foldered layout —
/// callers building this from a real path just pass whatever folder levels
/// they have; both are optional since a scan can start anywhere).
#[derive(Debug, Clone, Copy, Default)]
pub struct EpisodePathContext<'a> {
    pub season_dir_name: Option<&'a str>,
    pub series_dir_name: Option<&'a str>,
}

static SXXEYY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bs(\d{1,2})\s?e(\d{1,4})(?:[\s-]?e(\d{1,4}))?\b").unwrap());
static NXM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{1,4})\b").unwrap());
static SEASON_EPISODE_WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bseason\s?(\d{1,2})\b.*?\bepisode\s?(\d{1,4})\b").unwrap());
static EPISODE_ONLY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\be(?:p(?:isode)?)?\s?(\d{1,4})\b").unwrap());
static SEASON_FOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^season\s?(\d{1,3})$|^s(\d{1,3})$").unwrap());
static SPECIALS_FOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^specials?$").unwrap());

/// Reads a season number out of a folder name, recognizing `"Season 01"`,
/// `"Season 1"`, `"S01"`, and `"Specials"`/`"Special"` (season 0, the common
/// convention for extras/specials that live outside any numbered
/// season). Returns `None` for a folder name that isn't a season marker at
/// all (the show's own root folder, for instance).
fn season_from_folder_name(name: &str) -> Option<i32> {
    let normalized = normalize_separators(name);
    if SPECIALS_FOLDER.is_match(&normalized) {
        return Some(0);
    }
    SEASON_FOLDER.captures(&normalized).and_then(|caps| {
        caps.get(1)
            .or_else(|| caps.get(2))
            .and_then(|m| m.as_str().parse::<i32>().ok())
    })
}

fn resolve_series_title(prefix: &str, series_dir_name: Option<&str>) -> String {
    let from_filename = tidy_title(prefix);
    if !from_filename.is_empty() {
        return from_filename;
    }
    series_dir_name.map(tidy_title).unwrap_or_default()
}

/// Parses `stem` (a video file's name without its extension) as a TV
/// episode, consulting `ctx`'s folder names only when the filename itself
/// carries no usable season/episode marker. Returns `None` when nothing in
/// the filename or its folder ancestry looks like an episode marker at
/// all — the caller should leave such a file for manual classification
/// rather than guess.
pub fn parse_episode(stem: &str, ctx: EpisodePathContext<'_>) -> Option<ParsedEpisode> {
    let normalized = normalize_separators(stem);

    if let Some(caps) = SXXEYY.captures(&normalized) {
        let season = caps[1].parse().ok()?;
        let episode = caps[2].parse().ok()?;
        let end_episode = caps.get(3).and_then(|m| m.as_str().parse().ok());
        let prefix = &normalized[..caps.get(0).unwrap().start()];
        return Some(ParsedEpisode {
            series_title: resolve_series_title(prefix, ctx.series_dir_name),
            season,
            episode,
            end_episode,
        });
    }

    if let Some(caps) = NXM.captures(&normalized) {
        let season = caps[1].parse().ok()?;
        let episode = caps[2].parse().ok()?;
        let prefix = &normalized[..caps.get(0).unwrap().start()];
        return Some(ParsedEpisode {
            series_title: resolve_series_title(prefix, ctx.series_dir_name),
            season,
            episode,
            end_episode: None,
        });
    }

    if let Some(caps) = SEASON_EPISODE_WORDS.captures(&normalized) {
        let season = caps[1].parse().ok()?;
        let episode = caps[2].parse().ok()?;
        let prefix = &normalized[..caps.get(0).unwrap().start()];
        return Some(ParsedEpisode {
            series_title: resolve_series_title(prefix, ctx.series_dir_name),
            season,
            episode,
            end_episode: None,
        });
    }

    // No marker in the filename at all: fall back to a season number from
    // the season folder, paired with a bare episode marker in the
    // filename. Both must be present — a season folder alone with no
    // recognizable episode number in the filename still isn't enough to
    // place the file.
    //
    // In this branch, unlike the marker-in-filename branches above, the
    // text before the episode marker is an episode title ("Episode Title -
    // E02.mkv"), not a series name — the whole reason this convention needs
    // a season folder to supply the season is that the series name isn't
    // repeated in the filename at all. So the series folder name is always
    // preferred here, with the filename's own prefix used only when no
    // series folder was given.
    let season = ctx.season_dir_name.and_then(season_from_folder_name)?;
    let caps = EPISODE_ONLY.captures(&normalized)?;
    let episode = caps[1].parse().ok()?;
    let series_title = match ctx.series_dir_name {
        Some(dir) => tidy_title(dir),
        None => {
            let prefix = &normalized[..caps.get(0).unwrap().start()];
            resolve_series_title(prefix, None)
        }
    };
    Some(ParsedEpisode {
        series_title,
        season,
        episode,
        end_episode: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_CTX: EpisodePathContext<'static> = EpisodePathContext {
        season_dir_name: None,
        series_dir_name: None,
    };

    #[test]
    fn standard_sxxeyy_marker() {
        let result = parse_episode("Show Name S01E02", NO_CTX);
        assert_eq!(
            result,
            Some(ParsedEpisode {
                series_title: "Show Name".into(),
                season: 1,
                episode: 2,
                end_episode: None
            })
        );
    }

    #[test]
    fn dotted_sxxeyy_with_episode_title_and_quality_tag() {
        let result = parse_episode("Show.Name.S01.E02.The.Episode.Title.1080p.WEB-DL", NO_CTX);
        let parsed = result.unwrap();
        assert_eq!(parsed.season, 1);
        assert_eq!(parsed.episode, 2);
        assert_eq!(parsed.series_title, "Show Name");
    }

    #[test]
    fn multi_episode_directly_concatenated() {
        let result = parse_episode("Show Name S01E02E03", NO_CTX);
        assert_eq!(
            result,
            Some(ParsedEpisode {
                series_title: "Show Name".into(),
                season: 1,
                episode: 2,
                end_episode: Some(3)
            })
        );
    }

    #[test]
    fn multi_episode_dash_separated() {
        let result = parse_episode("Show Name S01E02-E03", NO_CTX);
        let parsed = result.unwrap();
        assert_eq!((parsed.episode, parsed.end_episode), (2, Some(3)));
    }

    #[test]
    fn trailing_year_after_sxxeyy_is_never_read_as_an_end_episode() {
        // A dated special ("...S01E02 2020.mkv") must not have "2020" read
        // as an end-episode of a bogus multi-episode range.
        let result = parse_episode("Show Name S01E02 2020", NO_CTX);
        let parsed = result.unwrap();
        assert_eq!(parsed.end_episode, None);
    }

    #[test]
    fn classic_nxm_format() {
        let result = parse_episode("Show Name 1x02", NO_CTX);
        assert_eq!(
            result,
            Some(ParsedEpisode {
                series_title: "Show Name".into(),
                season: 1,
                episode: 2,
                end_episode: None
            })
        );
    }

    #[test]
    fn nxm_format_is_not_confused_with_a_bare_resolution_tag() {
        // "1920x1080" must never parse as season 1920 / episode 1080 — the
        // season capture group is capped at two digits, which "1920"
        // (four digits) can never satisfy.
        let result = parse_episode("Show Name 1920x1080", NO_CTX);
        assert_eq!(result, None);
    }

    #[test]
    fn spelled_out_season_and_episode() {
        let result = parse_episode("Show Name Season 1 Episode 2", NO_CTX);
        let parsed = result.unwrap();
        assert_eq!((parsed.season, parsed.episode), (1, 2));
    }

    #[test]
    fn season_folder_plus_bare_episode_marker_in_filename() {
        let ctx = EpisodePathContext {
            season_dir_name: Some("Season 01"),
            series_dir_name: Some("Show Name"),
        };
        let result = parse_episode("Episode Title - E02", ctx);
        assert_eq!(
            result,
            Some(ParsedEpisode {
                series_title: "Show Name".into(),
                season: 1,
                episode: 2,
                end_episode: None
            })
        );
    }

    #[test]
    fn specials_folder_maps_to_season_zero() {
        let ctx = EpisodePathContext {
            season_dir_name: Some("Specials"),
            series_dir_name: Some("Show Name"),
        };
        let result = parse_episode("A Behind The Scenes Special - E01", ctx);
        let parsed = result.unwrap();
        assert_eq!(parsed.season, 0);
    }

    #[test]
    fn series_title_falls_back_to_series_folder_when_filename_has_no_prefix() {
        let ctx = EpisodePathContext {
            season_dir_name: None,
            series_dir_name: Some("My Favorite Show"),
        };
        let result = parse_episode("S01E02", ctx);
        assert_eq!(result.unwrap().series_title, "My Favorite Show");
    }

    #[test]
    fn no_season_folder_and_no_marker_in_filename_is_unrecognized() {
        let ctx = EpisodePathContext {
            season_dir_name: Some("Show Name"),
            series_dir_name: Some("Show Name"),
        };
        let result = parse_episode("Some Episode Title With No Numbers", ctx);
        assert_eq!(result, None);
    }

    #[test]
    fn plain_filename_with_no_context_and_no_marker_is_unrecognized() {
        assert_eq!(parse_episode("Just A Title", NO_CTX), None);
    }
}
