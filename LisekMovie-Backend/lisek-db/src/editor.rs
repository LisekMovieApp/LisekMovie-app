//! Faza 20: the metadata editor's writes for films, whole series and single
//! episodes, and manual artwork.
//!
//! What a field means depends on the scope:
//! - a film: its own title, original title, year, overview;
//! - a series: the same fields, written to every episode of the series
//!   (an episode row carries the series' title, it is how episodes are
//!   grouped), the series row itself renamed in place so its id and every
//!   reference to it stay;
//! - an episode: `provider_data.episode.{title, overview, air_date}`, the
//!   data the season page shows. The episode's `title` column stays the
//!   series title.
//!
//! Every field an administrator changes is added to `locked_fields`, so a
//! rescan (which takes the title from the file name) or a provider refresh
//! does not quietly undo the edit.
use uuid::Uuid;

use crate::{DbError, PgPool};

/// Lock names of fields shared by a whole series (or a film).
pub const SERIES_LEVEL_FIELDS: &[&str] = &[
    "title",
    "original_title",
    "year",
    "overview",
    "poster",
    "backdrop",
    "logo",
];
/// Lock names of one episode's own fields.
pub const EPISODE_LEVEL_FIELDS: &[&str] = &["episode_title", "episode_overview", "thumb"];
/// Artwork kinds an administrator can replace.
pub const ARTWORK_KINDS: &[&str] = &["poster", "backdrop", "logo", "thumb"];

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("no such item")]
    NotFound,
    #[error("this item is not an episode of a series")]
    NotAnEpisode,
    #[error("another series in this library already has that title")]
    TitleTaken,
    #[error(transparent)]
    Database(#[from] DbError),
}
impl From<sqlx::Error> for EditError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err.into())
    }
}

/// Fields shared by a film or a whole series.
pub struct SharedFields<'a> {
    pub title: &'a str,
    pub original_title: Option<&'a str>,
    pub year: Option<i32>,
    pub overview: Option<&'a str>,
    pub metadata_locked: bool,
    pub forced_provider: Option<&'a str>,
    /// Series-level lock names only (others are ignored).
    pub locked_fields: &'a [String],
    pub age_rating: Option<i32>,
}

/// One episode's own fields.
pub struct EpisodeFields<'a> {
    pub title: Option<&'a str>,
    pub overview: Option<&'a str>,
    pub air_date: Option<&'a str>,
    /// Episode-level lock names only (others are ignored).
    pub locked_fields: &'a [String],
}

/// The series and episode side of an item, for the editor.
#[derive(Debug, Clone, Default, serde::Serialize, utoipa::ToSchema)]
pub struct EpisodeContext {
    pub series_id: Option<Uuid>,
    pub series_title: Option<String>,
    pub episode_count: Option<i32>,
    pub episode_title: Option<String>,
    pub episode_overview: Option<String>,
    pub episode_air_date: Option<String>,
}

pub async fn episode_context(pool: &PgPool, item_id: Uuid) -> Result<EpisodeContext, DbError> {
    type Row = (Option<Uuid>, Option<String>, Option<i32>, Option<String>, Option<String>, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT s.id, s.title, s.episode_count, \
                m.metadata->'provider_data'->'episode'->>'title', \
                m.metadata->'provider_data'->'episode'->>'overview', \
                m.metadata->'provider_data'->'episode'->>'air_date' \
         FROM media_items m \
         LEFT JOIN episodes e ON e.media_item_id = m.id \
         LEFT JOIN series s ON s.id = e.series_id \
         WHERE m.id = $1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    Ok(row
        .map(|r| EpisodeContext {
            series_id: r.0,
            series_title: r.1,
            episode_count: r.2,
            episode_title: r.3,
            episode_overview: r.4,
            episode_air_date: r.5,
        })
        .unwrap_or_default())
}

fn only(names: &[String], allowed: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = names
        .iter()
        .filter(|n| allowed.contains(&n.as_str()))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The `SET` list shared by film and series writes. `$1` is the id array,
/// `$2`..`$10` the shared fields, `$11` the lock names this scope keeps
/// from each row (the other scope's).
const SHARED_SET: &str = r#"
        title = $2,
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
        locked_fields = ARRAY(
            SELECT DISTINCT f FROM unnest(locked_fields || $8::text[]) f
            WHERE f = ANY($8::text[]) OR f = ANY($10::text[])
            ORDER BY f
        ),
        updated_at = now()
"#;

async fn write_shared(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ids: &[Uuid],
    fields: &SharedFields<'_>,
    keep_from_rows: &[&str],
) -> Result<u64, EditError> {
    let locks = only(fields.locked_fields, SERIES_LEVEL_FIELDS);
    let keep: Vec<String> = keep_from_rows.iter().map(|s| s.to_string()).collect();
    let sql = format!("UPDATE media_items SET {SHARED_SET} WHERE id = ANY($1)");
    let result = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(ids)
        .bind(fields.title)
        .bind(fields.original_title)
        .bind(fields.year)
        .bind(fields.overview)
        .bind(fields.metadata_locked)
        .bind(fields.forced_provider)
        .bind(&locks)
        .bind(fields.age_rating)
        .bind(&keep)
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected())
}

/// A film (or any single item): its own fields.
pub async fn update_item(pool: &PgPool, item_id: Uuid, fields: &SharedFields<'_>) -> Result<(), EditError> {
    let mut tx = pool.begin().await?;
    // A film keeps no episode-level locks.
    if write_shared(&mut tx, &[item_id], fields, &[]).await? == 0 {
        return Err(EditError::NotFound);
    }
    tx.commit().await?;
    Ok(())
}

/// A whole series, given any of its episodes: renames the series row in
/// place and writes the shared fields to every episode. Returns how many
/// episodes were updated.
pub async fn update_series(pool: &PgPool, item_id: Uuid, fields: &SharedFields<'_>) -> Result<u64, EditError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(710030)").execute(&mut *tx).await?;
    let series: Option<(Uuid, Uuid, String)> = sqlx::query_as(
        "SELECT s.id, s.library_id, s.title FROM episodes e JOIN series s ON s.id = e.series_id \
         WHERE e.media_item_id = $1",
    )
    .bind(item_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((series_id, library_id, old_title)) = series else {
        let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM media_items WHERE id = $1")
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await?;
        return Err(if exists.is_some() { EditError::NotAnEpisode } else { EditError::NotFound });
    };
    if fields.title != old_title {
        let (taken,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM series WHERE library_id = $1 AND title = $2 AND id <> $3)",
        )
        .bind(library_id)
        .bind(fields.title)
        .bind(series_id)
        .fetch_one(&mut *tx)
        .await?;
        if taken {
            return Err(EditError::TitleTaken);
        }
        // Renamed first: the episodes' new title then links them back to
        // this same row (`lisek_link_episodes` finds it by library and title).
        sqlx::query("UPDATE series SET title = $2, updated_at = now() WHERE id = $1")
            .bind(series_id)
            .bind(fields.title)
            .execute(&mut *tx)
            .await?;
    }
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT media_item_id FROM episodes WHERE series_id = $1")
        .bind(series_id)
        .fetch_all(&mut *tx)
        .await?;
    let updated = write_shared(&mut tx, &ids, fields, EPISODE_LEVEL_FIELDS).await?;
    tx.commit().await?;
    Ok(updated)
}

/// One episode's own title, overview and air date.
pub async fn update_episode(pool: &PgPool, item_id: Uuid, fields: &EpisodeFields<'_>) -> Result<(), EditError> {
    let locks = only(fields.locked_fields, EPISODE_LEVEL_FIELDS);
    let keep: Vec<String> = SERIES_LEVEL_FIELDS.iter().map(|s| s.to_string()).collect();
    let result = sqlx::query(
        r#"
        UPDATE media_items
        SET metadata = metadata || jsonb_build_object(
                'provider_data',
                COALESCE(metadata->'provider_data', '{}'::jsonb) || jsonb_build_object(
                    'episode',
                    COALESCE(metadata->'provider_data'->'episode', '{}'::jsonb)
                        || jsonb_build_object('title', $2::text, 'overview', $3::text, 'air_date', $4::text)
                )
            ),
            locked_fields = ARRAY(
                SELECT DISTINCT f FROM unnest(locked_fields || $5::text[]) f
                WHERE f = ANY($5::text[]) OR f = ANY($6::text[])
                ORDER BY f
            ),
            updated_at = now()
        WHERE id = $1 AND item_type = 'episode'
        "#,
    )
    .bind(item_id)
    .bind(fields.title)
    .bind(fields.overview)
    .bind(fields.air_date)
    .bind(&locks)
    .bind(&keep)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        let item_type: Option<String> = sqlx::query_scalar("SELECT item_type FROM media_items WHERE id = $1")
            .bind(item_id)
            .fetch_optional(pool)
            .await?;
        return Err(if item_type.is_some() { EditError::NotAnEpisode } else { EditError::NotFound });
    }
    Ok(())
}

/// Records a manual artwork replacement on `ids`: the kind is locked
/// against provider refreshes and older media-folder files of that kind
/// are ignored from `at_ms` on (see [`artwork_source`]).
pub async fn record_artwork_override(pool: &PgPool, ids: &[Uuid], kind: &str, at_ms: i64) -> Result<(), DbError> {
    sqlx::query(
        r#"
        UPDATE media_items
        SET metadata = metadata || jsonb_build_object(
                'artwork_overrides',
                COALESCE(metadata->'artwork_overrides', '{}'::jsonb) || jsonb_build_object($2::text, $3::bigint)
            ),
            locked_fields = CASE WHEN $2 = ANY(locked_fields) THEN locked_fields
                                 ELSE locked_fields || $2::text END,
            updated_at = now()
        WHERE id = ANY($1)
        "#,
    )
    .bind(ids)
    .bind(kind)
    .bind(at_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// The item's file path and the moment from which media-folder files of
/// `kind` count: the later of the last Fix match and the last manual
/// replacement of that kind.
pub async fn artwork_source(pool: &PgPool, item_id: Uuid, kind: &str) -> Result<Option<(String, Option<i64>)>, DbError> {
    Ok(sqlx::query_as(
        "SELECT file_path, \
                GREATEST((metadata->>'artwork_reset_at')::bigint, \
                         (metadata->'artwork_overrides'->>$2)::bigint) \
         FROM media_items WHERE id = $1",
    )
    .bind(item_id)
    .bind(kind)
    .fetch_optional(pool)
    .await?)
}

/// Lock names that apply to an item during a provider refresh: its own,
/// plus the series-level ones any other episode of its series carries
/// (a new episode must not bring back artwork replaced on the series).
pub async fn effective_locked_fields(pool: &PgPool, item_id: Uuid) -> Result<Vec<String>, DbError> {
    let series_level: Vec<String> = SERIES_LEVEL_FIELDS.iter().map(|s| s.to_string()).collect();
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(array_agg(DISTINCT f ORDER BY f), '{}') FROM ( \
             SELECT unnest(locked_fields) AS f FROM media_items WHERE id = $1 \
             UNION \
             SELECT unnest(m.locked_fields) FROM episodes e \
             JOIN episodes sib ON sib.series_id = e.series_id \
             JOIN media_items m ON m.id = sib.media_item_id \
             WHERE e.media_item_id = $1 \
         ) l WHERE l.f = ANY($2) OR l.f IN (SELECT unnest(locked_fields) FROM media_items WHERE id = $1)",
    )
    .bind(item_id)
    .bind(&series_level)
    .fetch_one(pool)
    .await?)
}

/// Milliseconds of the item's last change: a cache-buster for previews.
pub async fn item_version(pool: &PgPool, item_id: Uuid) -> Result<Option<i64>, DbError> {
    Ok(sqlx::query_scalar("SELECT (extract(epoch FROM updated_at) * 1000)::bigint FROM media_items WHERE id = $1")
        .bind(item_id)
        .fetch_optional(pool)
        .await?)
}
