//! Domain types and business logic: libraries, media items, users.
//!
//! Faza 2 added `User` — enough to support the First-Run Wizard creating the
//! first admin account. Faza 5 adds `Library`, and later — once the scanner
//! and metadata providers actually have rows worth reading back out — adds
//! `MediaItem` too, for the home-feed/browse read side.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// A local account. Never carries the password hash — that stays inside the
/// db/auth layers and must never be serialized back to a client.
///
/// Deliberately has no sqlx dependency or derive: lisek-db maps database rows
/// onto this struct field by field, so this crate stays free of any notion
/// of Postgres or SQL (see the architecture plan's crate boundaries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub is_admin: bool,
    pub must_change_password: bool,
    pub disabled: bool,
    pub ui_language: String,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A library: one of the admin's configured content roots (a movies
/// collection, a TV shows collection, ...) plus the on-disk paths the Faza 5
/// scanner (`lisek-library`) walks to find its files.
///
/// `library_type` is a plain string here rather than an enum — this crate
/// stays free of any dependency on `lisek-library`, which is where the
/// scanner-facing `LibraryType` enum and its two canonical values
/// (`"movies"` / `"tv_shows"`) actually live; `lisek-db` validates against
/// those same values when creating a library, so an invalid type can never
/// reach this struct via the normal path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct Library {
    pub id: Uuid,
    pub name: String,
    pub library_type: String,
    pub sort_order: i32,
    pub root_paths: Vec<String>,
    /// Whether a metadata provider match's poster/backdrop get written
    /// directly next to the video file rather
    /// than only into the app's own local cache. Defaults to `true` at the
    /// database level (Faza 5 round 2's migration) — most self-hosters
    /// expect this, and it keeps the library portable to another tool
    /// later. See `lisek_metadata::image_cache::save_image` for what
    /// happens when the media folder itself isn't writable.
    pub save_metadata_in_media_folder: bool,
    pub created_at: DateTime<Utc>,
}

/// One playable item: a movie, or a single TV episode. `lisek-db`'s write
/// path (the scanner-persistence and metadata-enrichment functions) never
/// builds one of these — it works directly against `media_items` rows —
/// this is only what the read side (the home feed, and later a full browse
/// view) hands back to `lisek-server`.
///
/// `item_type` stays a plain string for the same reason `Library::
/// library_type` does: `lisek-library`'s scan-result enum is what actually
/// produces `"movie"` / `"episode"`, and this crate doesn't depend on it.
///
/// `year`/`overview`/`season`/`episode` are `None` until a metadata
/// provider (or, for season/episode, the scanner's own filename parsing)
/// has filled them in — a freshly-scanned, not-yet-enriched item is a
/// perfectly valid `MediaItem`, just a sparser one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct MediaItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub item_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-core");
    }

    #[test]
    fn user_serializes_without_a_password_field() {
        let user = User {
            id: Uuid::nil(),
            username: "admin".to_string(),
            is_admin: true,
            must_change_password: false,
            disabled: false,
            ui_language: "en".to_string(),
            email: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_value(&user).unwrap();
        assert!(json.get("password").is_none());
        assert!(json.get("password_hash").is_none());
        assert!(json.get("temporary_password").is_none());
        assert_eq!(json["must_change_password"], false);
    }
}

pub mod playback_settings;
