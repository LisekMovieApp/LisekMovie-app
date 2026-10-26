//! Where a series' artwork lives on disk (Faza 10), the way Jellyfin lays
//! it out: everything in the series' own folder, season folders keep only
//! the videos.
//!
//! ```text
//! Shows/13 Posterunek (1997)/        <- series folder
//!     poster.jpg  backdrop.jpg  logo.png
//!     season01-poster.jpg  season02-poster.jpg  season-specials-poster.jpg
//!     Season 01/  13.Posterunek.S01E01.mkv ...
//! ```
//!
//! Episodes sitting directly in the series folder (no season folders) work
//! the same: their parent is the series folder.

use std::path::{Path, PathBuf};

/// The season number a folder name stands for: `Season 01`, `Sezon 2`,
/// `Staffel 3`, `Saison 4`, `Temporada 5`, `Series 6`, `S07`, and
/// `Specials` / `Odcinki specjalne` (0). `None` for anything else.
pub fn season_folder_number(name: &str) -> Option<i32> {
    let lower = name.trim().to_lowercase();
    if matches!(
        lower.as_str(),
        "specials" | "special" | "odcinki specjalne" | "extras specials"
    ) {
        return Some(0);
    }
    let rest = [
        "season",
        "sezon",
        "staffel",
        "saison",
        "temporada",
        "stagione",
        "series",
        "seizoen",
        "serie",
    ]
    .iter()
    .find_map(|word| lower.strip_prefix(word))
    .or_else(|| {
        let after = lower.strip_prefix('s')?;
        after
            .chars()
            .next()
            .filter(char::is_ascii_digit)
            .map(|_| after)
    })?;
    let digits = rest.trim_start_matches([' ', '.', '_', '-']).trim();
    if digits.is_empty() || digits.len() > 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The series folder of an episode file: the parent of its season folder,
/// or its own folder when it is not in one.
pub fn series_folder(episode_file: &Path) -> Option<PathBuf> {
    let parent = episode_file.parent()?;
    let is_season = parent
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(season_folder_number)
        .is_some();
    match (is_season, parent.parent()) {
        (true, Some(series)) if series.file_name().is_some() => Some(series.to_path_buf()),
        _ => Some(parent.to_path_buf()),
    }
}

/// Jellyfin's season poster name: `season01-poster.jpg`,
/// `season-specials-poster.jpg` for season 0.
pub fn season_poster_name(season: i32) -> String {
    if season == 0 {
        "season-specials-poster.jpg".to_owned()
    } else {
        format!("season{season:02}-poster.jpg")
    }
}

/// The local cache copy of a season poster (library without media-folder
/// saving, or an unwritable media folder):
/// `config/metadata/series/<series id>/season01-poster.jpg`.
pub fn season_poster_cache_path(config_dir: &Path, series_id: &str, season: i32) -> PathBuf {
    config_dir
        .join("metadata")
        .join("series")
        .join(series_id)
        .join(season_poster_name(season))
}

/// An episode's still (from the provider), always in the local cache —
/// season folders stay video-only: `config/metadata/episodes/<id>/still.jpg`.
pub fn episode_still_path(config_dir: &Path, item_id: &str) -> PathBuf {
    config_dir
        .join("metadata")
        .join("episodes")
        .join(item_id)
        .join("still.jpg")
}

/// A frame cut from the episode itself when the provider has no still.
pub fn episode_frame_path(config_dir: &Path, item_id: &str) -> PathBuf {
    config_dir
        .join("metadata")
        .join("episodes")
        .join(item_id)
        .join("frame.jpg")
}

/// Artwork LisekMovie used to write into season folders before Faza 10.
pub const LEGACY_SEASON_FOLDER_FILES: &[&str] = &["poster.jpg", "backdrop.jpg", "logo.png"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn season_folders_are_recognised_in_several_languages() {
        for (name, number) in [
            ("Season 01", Some(1)),
            ("season 1", Some(1)),
            ("Sezon 2", Some(2)),
            ("Staffel 3", Some(3)),
            ("Saison.04", Some(4)),
            ("S05", Some(5)),
            ("s1", Some(1)),
            ("Season_2010", Some(2010)),
            ("Specials", Some(0)),
            ("Odcinki specjalne", Some(0)),
            ("Season", None),
            ("Seasoned Chef", None),
            ("Shows", None),
            ("Sherlock (2010)", None),
            ("Super 8", None),
        ] {
            assert_eq!(season_folder_number(name), number, "{name}");
        }
    }

    #[test]
    fn the_series_folder_is_above_a_season_folder() {
        let in_season = Path::new("/media/Shows/13 Posterunek (1997)/Season 01/S01E01.mkv");
        assert_eq!(
            series_folder(in_season).unwrap(),
            Path::new("/media/Shows/13 Posterunek (1997)")
        );
        let flat = Path::new("/media/Shows/Sherlock/Sherlock.S01E01.mkv");
        assert_eq!(
            series_folder(flat).unwrap(),
            Path::new("/media/Shows/Sherlock")
        );
        assert_eq!(season_poster_name(1), "season01-poster.jpg");
        assert_eq!(season_poster_name(12), "season12-poster.jpg");
        assert_eq!(season_poster_name(0), "season-specials-poster.jpg");
    }
}
