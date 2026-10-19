//! Recursive filesystem walk over a library's root paths: finds video
//! files, skips folder/file conventions that mark non-primary content
//! (extras, behind-the-scenes, sample clips), and captures enough of each file's containing
//! folder names for the episode parser's folder-based fallback (see
//! [`crate::naming::episode`]).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use walkdir::{DirEntry, WalkDir};

/// Container/extension formats accepted as a "video file". Not exhaustive
/// (no `.iso`/`.vob`/`.m2ts` DVD- or BluRay-folder-structure support yet —
/// a known, documented Faza 5 simplification, same as the naming module's),
/// but covers the file types the overwhelming majority of a real library
/// will actually contain.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "ts", "m2ts",
];

/// Folder names (case-insensitive) whose contents are extras/bonus
/// material rather than primary media — a common "extras" folder
/// convention. Their contents are skipped entirely rather than
/// mis-identified as a movie or episode in their own right.
const EXTRAS_DIR_NAMES: &[&str] = &[
    "extras",
    "behind the scenes",
    "deleted scenes",
    "featurettes",
    "interviews",
    "scenes",
    "shorts",
    "trailers",
    "samples",
];

/// One video file found on disk, with the immediate parent folder name
/// (a season folder, in a season-foldered TV layout) and the folder above
/// that (typically the show's own root folder) captured alongside it —
/// exactly what [`crate::naming::episode::parse_episode`]'s folder-based
/// fallback needs, without this crate's scanner having to know anything
/// about naming conventions itself.
#[derive(Debug, Clone)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub season_dir_name: Option<String>,
    pub series_dir_name: Option<String>,
    /// Size and modification time let a rescan skip ffprobe for files
    /// that have not changed. `None` when the OS cannot report them.
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

fn has_video_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| VIDEO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn is_hidden(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|name| name.starts_with('.'))
        .unwrap_or(false)
}

fn is_extras_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .map(|name| EXTRAS_DIR_NAMES.contains(&name.to_lowercase().as_str()))
            .unwrap_or(false)
}

/// A "sample" clip bundled alongside the real file — recognized by the
/// word "sample" appearing anywhere in the name (as its own
/// word, not as a substring of something else) and excluded from the
/// library on name alone (no size heuristic).
fn is_sample_file(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|stem| {
            crate::naming::clean::normalize_separators(stem)
                .split(' ')
                .any(|word| word.eq_ignore_ascii_case("sample"))
        })
        .unwrap_or(false)
}

fn dir_name(path: Option<&Path>) -> Option<String> {
    path.and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(str::to_string)
}

/// Recursively walks every path in `root_paths`, returning every video
/// file found (skipping hidden entries, known extras folders, and sample
/// clips). A root path that doesn't exist or can't be read is skipped
/// rather than aborting the whole scan — see the module doc comment.
pub fn scan_paths<P: AsRef<Path>>(root_paths: &[P]) -> Vec<ScannedFile> {
    scan_paths_with_progress(root_paths, |_| {})
}

/// Same walk as [`scan_paths`], calling `on_file` with the running count
/// after every accepted video file. Blocking: run it on a blocking thread
/// (`tokio::task::spawn_blocking`), never directly on an async worker.
pub fn scan_paths_with_progress<P: AsRef<Path>>(
    root_paths: &[P],
    mut on_file: impl FnMut(usize),
) -> Vec<ScannedFile> {
    let mut results = Vec::new();

    for root in root_paths {
        let root = root.as_ref();
        if !root.exists() {
            continue;
        }

        let walker = WalkDir::new(root).into_iter().filter_entry(|entry| {
            let is_root = entry.depth() == 0;
            is_root || (!is_hidden(entry) && !is_extras_dir(entry))
        });

        for entry in walker.filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if !has_video_extension(path) || is_sample_file(path) {
                continue;
            }

            let season_dir_name = dir_name(path.parent());
            let series_dir_name = dir_name(path.parent().and_then(Path::parent));
            // On Windows walkdir answers this from the directory listing
            // itself; elsewhere it is one stat per file.
            let metadata = entry.metadata().ok();

            results.push(ScannedFile {
                path: path.to_path_buf(),
                season_dir_name,
                series_dir_name,
                size_bytes: metadata.as_ref().map(|m| m.len()),
                modified: metadata.and_then(|m| m.modified().ok()),
            });
            on_file(results.len());
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_video_files_and_ignores_non_video_ones() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie.mkv"), b"").unwrap();
        fs::write(dir.path().join("readme.txt"), b"").unwrap();
        fs::write(dir.path().join("poster.jpg"), b"").unwrap();

        let results = scan_paths(&[dir.path()]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path.file_name().unwrap(), "movie.mkv");
    }

    #[test]
    fn recurses_into_subfolders_and_captures_parent_folder_names() {
        let dir = tempfile::tempdir().unwrap();
        let series_dir = dir.path().join("My Show");
        let season_dir = series_dir.join("Season 01");
        fs::create_dir_all(&season_dir).unwrap();
        fs::write(season_dir.join("episode.mkv"), b"").unwrap();

        let results = scan_paths(&[dir.path()]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].season_dir_name.as_deref(), Some("Season 01"));
        assert_eq!(results[0].series_dir_name.as_deref(), Some("My Show"));
    }

    #[test]
    fn skips_hidden_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".hidden.mkv"), b"").unwrap();
        let hidden_dir = dir.path().join(".hidden_dir");
        fs::create_dir_all(&hidden_dir).unwrap();
        fs::write(hidden_dir.join("movie.mkv"), b"").unwrap();

        assert_eq!(scan_paths(&[dir.path()]).len(), 0);
    }

    #[test]
    fn skips_known_extras_folders_entirely() {
        let dir = tempfile::tempdir().unwrap();
        let extras_dir = dir.path().join("Behind The Scenes");
        fs::create_dir_all(&extras_dir).unwrap();
        fs::write(extras_dir.join("clip.mkv"), b"").unwrap();
        fs::write(dir.path().join("Main Movie (2020).mkv"), b"").unwrap();

        let results = scan_paths(&[dir.path()]);
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].path.file_name().unwrap(),
            "Main Movie (2020).mkv"
        );
    }

    #[test]
    fn skips_sample_clips() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("movie-sample.mkv"), b"").unwrap();
        fs::write(dir.path().join("sample.mkv"), b"").unwrap();
        fs::write(dir.path().join("movie.mkv"), b"").unwrap();

        let results = scan_paths(&[dir.path()]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path.file_name().unwrap(), "movie.mkv");
    }

    #[test]
    fn a_missing_root_path_is_skipped_rather_than_erroring() {
        let results = scan_paths(&["/this/path/definitely/does/not/exist"]);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn extension_matching_is_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("Movie.MKV"), b"").unwrap();

        assert_eq!(scan_paths(&[dir.path()]).len(), 1);
    }
}
