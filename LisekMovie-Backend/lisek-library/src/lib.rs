//! Folder scanning, file detection, matching files to media items.
//!
//! Faza 5 populates this crate: [`naming`] holds the filename parsers
//! (movie title/year, episode series/season/episode), and
//! [`scanner`] walks a library's root paths on disk to find video files
//! worth parsing. Orchestration — combining a scanned path with a parse
//! result into something ready to persist — lives at this crate's root so
//! `lisek-db` and `lisek-server` have one place to import from.

pub mod naming;
pub mod scanner;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub use naming::{EpisodePathContext, ParsedEpisode, ParsedMovie, parse_episode, parse_movie_name};
pub use scanner::{ScannedFile, scan_paths, scan_paths_with_progress};

/// Which parser a library's files should go through. Mirrors the
/// `libraries.library_type` column — `"movies"` / `"tv_shows"` are this
/// crate's canonical values for it (see [`LibraryType::as_db_str`] /
/// [`LibraryType::from_db_str`]); nothing wrote that column before Faza 5,
/// so this is where the convention is established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryType {
    Movies,
    TvShows,
}

impl LibraryType {
    pub fn as_db_str(self) -> &'static str {
        match self {
            LibraryType::Movies => "movies",
            LibraryType::TvShows => "tv_shows",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "movies" => Some(LibraryType::Movies),
            "tv_shows" => Some(LibraryType::TvShows),
            _ => None,
        }
    }
}

/// One scanned-and-parsed movie, ready to be matched against (or inserted
/// into) `media_items`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedMovie {
    pub file_path: PathBuf,
    pub title: String,
    pub year: Option<i32>,
    pub file: FileFacts,
}

/// On-disk facts about a scanned file, carried through to `media_items`
/// so a rescan can tell whether a stored ffprobe result is still valid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FileFacts {
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

/// One scanned-and-parsed TV episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedEpisode {
    pub file_path: PathBuf,
    pub series_title: String,
    pub season: i32,
    pub episode: i32,
    pub end_episode: Option<i32>,
    pub file: FileFacts,
}

/// The outcome of scanning and parsing one video file. `Unrecognized`
/// carries the path forward rather than dropping it silently, so a caller
/// can surface "N files couldn't be identified" to the admin instead of
/// files quietly vanishing from the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanResult {
    Movie(ScannedMovie),
    Episode(ScannedEpisode),
    Unrecognized(PathBuf),
}

/// Scans every root path in `root_paths` for video files and parses each
/// one according to `library_type`. `now_year` bounds movie-year
/// plausibility (see [`naming::movie::parse_movie_name`]) — pass the
/// current calendar year.
///
/// Directories that don't exist or can't be read are skipped rather than
/// failing the whole scan (a library can list a root path on removable or
/// network storage that's temporarily offline) — see [`scanner::scan_paths`].
pub fn scan_library<P: AsRef<Path>>(
    root_paths: &[P],
    library_type: LibraryType,
    now_year: i32,
) -> Vec<ScanResult> {
    scan_library_with_progress(root_paths, library_type, now_year, |_| {})
}

/// [`scan_library`] with a callback receiving the number of video files
/// found so far. Blocking — see [`scanner::scan_paths_with_progress`].
pub fn scan_library_with_progress<P: AsRef<Path>>(
    root_paths: &[P],
    library_type: LibraryType,
    now_year: i32,
    on_file: impl FnMut(usize),
) -> Vec<ScanResult> {
    scan_paths_with_progress(root_paths, on_file)
        .into_iter()
        .map(|file| classify(file, library_type, now_year))
        .collect()
}

fn classify(file: ScannedFile, library_type: LibraryType, now_year: i32) -> ScanResult {
    let stem = file
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let facts = FileFacts {
        size_bytes: file.size_bytes,
        modified: file.modified,
    };

    match library_type {
        LibraryType::Movies => {
            let parsed = parse_movie_name(stem, now_year);
            if parsed.title.is_empty() {
                ScanResult::Unrecognized(file.path)
            } else {
                ScanResult::Movie(ScannedMovie {
                    file_path: file.path,
                    title: parsed.title,
                    year: parsed.year,
                    file: facts,
                })
            }
        }
        LibraryType::TvShows => {
            let ctx = EpisodePathContext {
                season_dir_name: file.season_dir_name.as_deref(),
                series_dir_name: file.series_dir_name.as_deref(),
            };
            match parse_episode(stem, ctx) {
                Some(parsed) if !parsed.series_title.is_empty() => {
                    ScanResult::Episode(ScannedEpisode {
                        file_path: file.path,
                        series_title: parsed.series_title,
                        season: parsed.season,
                        episode: parsed.episode,
                        end_episode: parsed.end_episode,
                        file: facts,
                    })
                }
                _ => ScanResult::Unrecognized(file.path),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-library");
    }

    #[test]
    fn library_type_round_trips_through_its_db_string() {
        assert_eq!(
            LibraryType::from_db_str("movies"),
            Some(LibraryType::Movies)
        );
        assert_eq!(
            LibraryType::from_db_str("tv_shows"),
            Some(LibraryType::TvShows)
        );
        assert_eq!(LibraryType::from_db_str("bogus"), None);
        assert_eq!(LibraryType::Movies.as_db_str(), "movies");
        assert_eq!(LibraryType::TvShows.as_db_str(), "tv_shows");
    }

    #[test]
    fn scan_library_classifies_movies_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Some Movie (2020).mkv"), b"").unwrap();
        fs::write(dir.path().join("not a video.txt"), b"").unwrap();

        let results = scan_library(&[dir.path()], LibraryType::Movies, 2026);
        assert_eq!(results.len(), 1);
        match &results[0] {
            ScanResult::Movie(m) => {
                assert_eq!(m.title, "Some Movie");
                assert_eq!(m.year, Some(2020));
            }
            other => panic!("expected a Movie result, got {other:?}"),
        }
    }

    #[test]
    fn scan_library_classifies_episodes_end_to_end_with_season_folder() {
        let dir = tempfile::tempdir().unwrap();
        let series_dir = dir.path().join("My Show");
        let season_dir = series_dir.join("Season 01");
        fs::create_dir_all(&season_dir).unwrap();
        fs::write(season_dir.join("My Show S01E01.mkv"), b"").unwrap();

        let results = scan_library(&[dir.path()], LibraryType::TvShows, 2026);
        assert_eq!(results.len(), 1);
        match &results[0] {
            ScanResult::Episode(e) => {
                assert_eq!(e.series_title, "My Show");
                assert_eq!(e.season, 1);
                assert_eq!(e.episode, 1);
            }
            other => panic!("expected an Episode result, got {other:?}"),
        }
    }

    #[test]
    fn scan_records_file_size_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("A (2001).mkv"), b"12345").unwrap();
        fs::write(dir.path().join("B (2002).mkv"), b"1").unwrap();

        let mut seen = Vec::new();
        let results =
            scan_library_with_progress(&[dir.path()], LibraryType::Movies, 2026, |n| seen.push(n));
        assert_eq!(seen, vec![1, 2]);
        let sizes: std::collections::BTreeSet<_> = results
            .iter()
            .map(|r| match r {
                ScanResult::Movie(m) => {
                    assert!(m.file.modified.is_some());
                    m.file.size_bytes.unwrap()
                }
                other => panic!("expected a movie, got {other:?}"),
            })
            .collect();
        assert_eq!(sizes, [1, 5].into_iter().collect());
    }

    #[test]
    fn scan_library_reports_unparseable_files_as_unrecognized_rather_than_dropping_them() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("mystery_file.mkv"), b"").unwrap();

        let results = scan_library(&[dir.path()], LibraryType::TvShows, 2026);
        assert_eq!(results.len(), 1);
        assert!(matches!(&results[0], ScanResult::Unrecognized(_)));
    }
}
