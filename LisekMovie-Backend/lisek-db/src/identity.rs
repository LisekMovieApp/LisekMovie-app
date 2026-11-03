//! Faza 22: an item's identity at the metadata plugins — the ids it was
//! matched with (`provider_data.tmdb_id`, `imdb_id` ...), and whether an
//! administrator touched it.
//!
//! Once an item has an id, enrichment asks the plugins by that id only and
//! never searches by title again: a title an administrator renamed ("Nowe
//! Przygody Różowej Pantery") must not be taken for a new search that
//! finds another film. The `provider_ids` lock (set by every save in the
//! metadata editor, Fix match and a replaced picture) also keeps the
//! stored ids exactly as they are.
use std::collections::BTreeMap;
use std::path::PathBuf;

use uuid::Uuid;

use crate::{DbError, EnrichTarget, PgPool};

/// Lock name: the stored ids are the administrator's.
pub const PROVIDER_IDS_LOCK: &str = "provider_ids";

/// Id kinds kept in `provider_data` as `<kind>_id`. Most are a plugin's
/// own key; `mal` (MyAnimeList) comes from AniList.
pub const ID_KINDS: &[&str] = &["tmdb", "tvdb", "imdb", "anidb", "anilist", "mal", "omdb", "imvdb"];

/// Kinds whose ids are numbers (stored as JSON numbers, like the plugins do).
pub const NUMERIC_KINDS: &[&str] = &["tmdb", "tvdb", "anidb", "anilist", "mal"];

/// What enrichment needs to know before asking any plugin.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredIdentity {
    /// The plugin of the last match (`metadata.provider`).
    pub provider: Option<String>,
    /// Kind (`tmdb`) → id (`"603"`, `"tt0133093"`).
    pub ids: BTreeMap<String, String>,
    /// `locked_fields` is not empty: an administrator edited the item.
    pub edited: bool,
    /// `provider_ids` is locked.
    pub ids_locked: bool,
}

/// The ids in a `provider_data` object, by kind.
pub fn ids_of(provider_data: &serde_json::Value) -> BTreeMap<String, String> {
    let mut ids = BTreeMap::new();
    for kind in ID_KINDS {
        let value = match provider_data.get(format!("{kind}_id")) {
            Some(serde_json::Value::Number(n)) => n.to_string(),
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().to_owned(),
            _ => continue,
        };
        ids.insert((*kind).to_owned(), value);
    }
    ids
}

/// An id as stored: a number for the numeric kinds, else a string.
pub fn id_value(kind: &str, id: &str) -> serde_json::Value {
    if NUMERIC_KINDS.contains(&kind)
        && let Ok(n) = id.parse::<u64>()
    {
        return serde_json::json!(n);
    }
    serde_json::json!(id)
}

/// Whether `id` is a valid id of `kind` (an administrator typed it).
pub fn valid_id(kind: &str, id: &str) -> bool {
    match kind {
        "imdb" | "omdb" => {
            id.len() <= 12 && id.starts_with("tt") && id.len() > 4 && id[2..].bytes().all(|b| b.is_ascii_digit())
        }
        k if NUMERIC_KINDS.contains(&k) => {
            (1..=12).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit()) && id.parse::<u64>().is_ok_and(|n| n > 0)
        }
        "imvdb" => {
            (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        }
        _ => false,
    }
}

pub async fn stored_identity(pool: &PgPool, item_id: Uuid) -> Result<StoredIdentity, DbError> {
    let row: Option<(Option<String>, Option<serde_json::Value>, Vec<String>)> = sqlx::query_as(
        "SELECT metadata->>'provider', metadata->'provider_data', locked_fields FROM media_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    Ok(row
        .map(|(provider, data, locked)| StoredIdentity {
            provider: provider.filter(|p| !p.is_empty()),
            ids: data.as_ref().map(ids_of).unwrap_or_default(),
            edited: !locked.is_empty(),
            ids_locked: locked.iter().any(|l| l == PROVIDER_IDS_LOCK),
        })
        .unwrap_or_default())
}

/// Replaces the ids of `items` (every known kind is rewritten: a kind not
/// in `ids` is removed), forgets which plugin matched last and locks the
/// ids. Returns how many items were written.
pub async fn set_provider_ids(
    pool: &PgPool,
    items: &[Uuid],
    ids: &BTreeMap<String, String>,
) -> Result<u64, DbError> {
    let keys: Vec<String> = ID_KINDS.iter().map(|k| format!("{k}_id")).collect();
    let mut fresh = serde_json::Map::new();
    for (kind, id) in ids {
        fresh.insert(format!("{kind}_id"), id_value(kind, id));
    }
    let result = sqlx::query(
        "UPDATE media_items SET \
             metadata = (metadata - 'provider') || jsonb_build_object('provider_data', \
                 (COALESCE(metadata->'provider_data', '{}'::jsonb) - $2::text[]) || $3::jsonb), \
             locked_fields = CASE WHEN $4 = ANY(locked_fields) THEN locked_fields \
                                  ELSE array_append(locked_fields, $4) END, \
             updated_at = now() \
         WHERE id = ANY($1)",
    )
    .bind(items)
    .bind(&keys)
    .bind(serde_json::Value::Object(fresh))
    .bind(PROVIDER_IDS_LOCK)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

/// Locks the ids of `items` (an administrator changed something).
pub async fn lock_provider_ids(pool: &PgPool, items: &[Uuid]) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE media_items SET locked_fields = array_append(locked_fields, $2) \
         WHERE id = ANY($1) AND NOT ($2 = ANY(locked_fields))",
    )
    .bind(items)
    .bind(PROVIDER_IDS_LOCK)
    .execute(pool)
    .await?;
    Ok(())
}

/// The items a change to `item_id`'s identity applies to: every episode of
/// its series, or the item itself.
pub async fn identity_scope(pool: &PgPool, item_id: Uuid) -> Result<Vec<Uuid>, DbError> {
    let episodes = crate::episodes::series_episode_ids(pool, item_id).await?;
    Ok(if episodes.is_empty() { vec![item_id] } else { episodes })
}

/// `(item, library, target, file, forced provider)` of `items` that are not
/// fully locked, grouped by series like the background refresh.
pub async fn items_for_refresh(
    pool: &PgPool,
    items: &[Uuid],
) -> Result<Vec<(Uuid, Uuid, EnrichTarget, PathBuf, Option<String>)>, DbError> {
    type Row = (Uuid, Uuid, String, String, Option<i32>, String, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, library_id, item_type, title, (metadata->>'year')::int, file_path, forced_provider \
         FROM media_items WHERE id = ANY($1) AND NOT metadata_locked \
         ORDER BY item_type, library_id, title, id",
    )
    .bind(items)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, library_id, item_type, title, year, file_path, forced)| {
            let target = if item_type == "movie" {
                EnrichTarget::Movie { title, year }
            } else {
                EnrichTarget::Series { title }
            };
            (id, library_id, target, PathBuf::from(file_path), forced)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_read_from_numbers_and_strings() {
        let data = serde_json::json!({"tmdb_id": 603, "imdb_id": "tt0133093", "tvdb_id": "", "overview": "x", "mal_id": "21"});
        let ids = ids_of(&data);
        assert_eq!(ids.get("tmdb").map(String::as_str), Some("603"));
        assert_eq!(ids.get("imdb").map(String::as_str), Some("tt0133093"));
        assert_eq!(ids.get("mal").map(String::as_str), Some("21"));
        assert!(!ids.contains_key("tvdb"));
        assert_eq!(id_value("tmdb", "603"), serde_json::json!(603));
        assert_eq!(id_value("imdb", "tt1"), serde_json::json!("tt1"));
    }

    #[test]
    fn typed_ids_are_checked_per_kind() {
        assert!(valid_id("imdb", "tt0133093"));
        assert!(!valid_id("imdb", "0133093"));
        assert!(!valid_id("imdb", "tt"));
        assert!(valid_id("tmdb", "603"));
        assert!(!valid_id("tmdb", "60a"));
        assert!(!valid_id("tmdb", "0"));
        assert!(valid_id("imvdb", "abc-1"));
        assert!(!valid_id("unknown", "1"));
    }
}
