//! Single-item reads and the admin metadata editor.
//!
//! `year`/`overview`/`season_number`/`episode_number` are generated columns
//! derived from `metadata` (migration 0013), so these reads no longer parse
//! JSONB per row.

use serde::Serialize;
use uuid::Uuid;

use crate::{DbError, PgPool};

/// One item's on-disk file path — used by the image-serving routes to
/// check for a poster/backdrop written next to the media itself before
/// falling back to the local config-dir cache (see `lisek-server`'s
/// `media.rs`). `None` when no such item exists at all, which the caller
/// treats the same as "no media-folder candidate, try the cache."
pub async fn get_media_item_file_path(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<String>, DbError> {
    let row: Option<(String,)> = sqlx::query_as("SELECT file_path FROM media_items WHERE id = $1")
        .bind(item_id)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|(file_path,)| file_path))
}

/// One item's full editable shape — what the metadata editor loads and
/// what it hands back for [`update_media_item`]. `original_title` reads
/// straight from `metadata->>'original_title'` (see the Faza
/// 5-completion migration's doc comment for why that's a JSONB key, not a
/// column) and `overview` from the same `provider_data` slot the home feed
/// already reads (a manual edit and a provider match both land there —
/// see [`update_media_item`]). `locked_fields` names, verbatim, whichever
/// of `"title"`/`"original_title"`/`"year"`/`"overview"` the admin has
/// individually protected — the same identifiers the editor's own
/// checkboxes use, so the frontend never needs a translation table.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MediaItemDetail {
    pub id: Uuid,
    pub library_id: Uuid,
    pub item_type: String,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub metadata_locked: bool,
    pub forced_provider: Option<String>,
    pub locked_fields: Vec<String>,
    /// An administrator's own age rating (parental controls), which wins
    /// over the provider's. `None`: the provider's value applies.
    pub age_rating: Option<i32>,
    /// What the provider says (shown next to the override in the editor).
    pub provider_age_rating: Option<i32>,
    pub certification: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AgeColumns {
    age_rating: Option<i32>,
    provider_age_rating: Option<i32>,
    certification: Option<String>,
}

type MediaItemDetailRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<String>,
    Option<i32>,
    Option<String>,
    Option<i32>,
    Option<i32>,
    bool,
    Option<String>,
    Vec<String>,
);

fn media_item_detail_from_row(row: MediaItemDetailRow) -> MediaItemDetail {
    MediaItemDetail {
        id: row.0,
        library_id: row.1,
        item_type: row.2,
        title: row.3,
        original_title: row.4,
        year: row.5,
        overview: row.6,
        season: row.7,
        episode: row.8,
        metadata_locked: row.9,
        forced_provider: row.10,
        locked_fields: row.11,
        age_rating: None,
        provider_age_rating: None,
        certification: None,
    }
}

/// Loads one item for the metadata editor. `None` when no such item
/// exists (a stale id from a deleted/re-scanned library) — the caller
/// turns that into a 404, same as every other single-resource lookup in
/// this crate.
pub async fn get_media_item_detail(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<MediaItemDetail>, DbError> {
    let row = sqlx::query_as::<_, MediaItemDetailRow>(
        r#"
        SELECT id, library_id, item_type, title,
               metadata->>'original_title' AS original_title,
               year,
               overview,
               season_number AS season,
               episode_number AS episode,
               metadata_locked, forced_provider, locked_fields
        FROM media_items
        WHERE id = $1
        "#,
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let ages = sqlx::query_as::<_, AgeColumns>(
        r#"
        SELECT CASE WHEN jsonb_typeof(metadata->'age_rating') = 'number'
                    THEN (metadata->>'age_rating')::numeric::int END AS age_rating,
               CASE WHEN jsonb_typeof(metadata->'provider_data'->'age_rating') = 'number'
                    THEN (metadata->'provider_data'->>'age_rating')::numeric::int END
                   AS provider_age_rating,
               metadata->'provider_data'->>'certification' AS certification
        FROM media_items WHERE id = $1
        "#,
    )
    .bind(item_id)
    .fetch_one(pool)
    .await?;

    let mut detail = media_item_detail_from_row(row);
    detail.age_rating = ages.age_rating;
    detail.provider_age_rating = ages.provider_age_rating;
    detail.certification = ages.certification;
    Ok(Some(detail))
}

/// The metadata editor's save action — an admin's own edit, so unlike
/// every automatic write in this module, this one is never gated on
/// `locked_fields` (those only ever protect a field from a *future
/// automatic* re-scan/refresh, never from the admin editing it directly
/// right now) and always applies every field, including setting
/// `metadata_locked`/`forced_provider`/`locked_fields` themselves — this
/// is the one place all three actually get written. `overview` lands in
/// the same `metadata->'provider_data'->>'overview'` slot a provider match
/// already uses (see [`attach_provider_metadata`]), so every reader keeps
/// working unchanged regardless of whether the current value came from an
/// admin or a provider; `tmdb_id`/`tvdb_id`/`provider`, if already present
/// in `provider_data`, survive this call untouched. Returns `false` (no
/// error) for an id that doesn't exist, same "not found, not a failure"
/// shape as [`delete_library`].
#[allow(clippy::too_many_arguments)]
pub async fn update_media_item(
    pool: &PgPool,
    item_id: Uuid,
    title: &str,
    original_title: Option<&str>,
    year: Option<i32>,
    overview: Option<&str>,
    metadata_locked: bool,
    forced_provider: Option<&str>,
    locked_fields: &[String],
    age_rating: Option<i32>,
) -> Result<bool, DbError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        r#"
        UPDATE media_items
        SET title = $2,
            metadata = (metadata - 'original_title' - 'year' - 'provider_data' - 'age_rating')
                || jsonb_build_object('original_title', $3::text)
                || jsonb_build_object('year', $4::int)
                || CASE WHEN $9::int IS NULL THEN '{}'::jsonb
                        ELSE jsonb_build_object('age_rating', $9::int) END
                || jsonb_build_object(
                    'provider_data',
                    COALESCE(metadata->'provider_data', '{}'::jsonb)
                        || jsonb_build_object('overview', $5::text)
                ),
            metadata_locked = $6,
            forced_provider = $7,
            locked_fields = $8,
            updated_at = now()
        WHERE id = $1
        RETURNING id
        "#,
    )
    .bind(item_id)
    .bind(title)
    .bind(original_title)
    .bind(year)
    .bind(overview)
    .bind(metadata_locked)
    .bind(forced_provider)
    .bind(locked_fields)
    .bind(age_rating)
    .fetch_optional(pool)
    .await?;

    Ok(row.is_some())
}

/// Every item in one library, for the Admin Panel's browse view that opens
/// the metadata editor — deliberately not paginated yet (a self-hosted
/// library's realistic size doesn't need it at Faza 5's scope; revisit if
/// that stops being true). `metadata_locked`/`metadata_fallback` ride
/// along so the browse list itself can show a lock/warning badge without a
/// second request per row.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MediaItemSummary {
    pub id: Uuid,
    pub item_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub metadata_locked: bool,
    pub metadata_fallback: bool,
}

type MediaItemSummaryRow = (
    Uuid,
    String,
    String,
    Option<i32>,
    Option<i32>,
    Option<i32>,
    bool,
    bool,
);

pub async fn list_media_items_for_library(
    pool: &PgPool,
    library_id: Uuid,
) -> Result<Vec<MediaItemSummary>, DbError> {
    let rows = sqlx::query_as::<_, MediaItemSummaryRow>(
        r#"
        SELECT id, item_type, title,
               year,
               season_number AS season,
               episode_number AS episode,
               metadata_locked, metadata_fallback
        FROM media_items
        WHERE library_id = $1
        ORDER BY title ASC
        "#,
    )
    .bind(library_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, item_type, title, year, season, episode, metadata_locked, metadata_fallback)| {
                MediaItemSummary {
                    id,
                    item_type,
                    title,
                    year,
                    season,
                    episode,
                    metadata_locked,
                    metadata_fallback,
                }
            },
        )
        .collect())
}

/// Every item currently flagged `metadata_fallback` — the Admin Panel's
/// warning view (see the Faza 5-completion roadmap item) lists these with
/// a "move to the media folder" action per row, so the admin doesn't have
/// to `SELECT` the database by hand to find out which items only ever got
/// their poster/backdrop into the local cache. `file_path` rides along so
/// the caller (`lisek-server`) can derive the exact media-folder
/// destination the same way `metadata_enrichment::media_folder_destination`
/// already does, without a second lookup.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct FallbackItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub title: String,
    pub file_path: String,
}

pub async fn list_fallback_items(pool: &PgPool) -> Result<Vec<FallbackItem>, DbError> {
    let rows: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
        r#"
        SELECT id, library_id, title, file_path
        FROM media_items
        WHERE metadata_fallback = true
        ORDER BY title ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, library_id, title, file_path)| FallbackItem {
            id,
            library_id,
            title,
            file_path,
        })
        .collect())
}
