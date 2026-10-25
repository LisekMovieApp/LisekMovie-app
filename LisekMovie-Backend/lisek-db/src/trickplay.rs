//! Trickplay sheet bookkeeping; `lisek_playback::trickplay` makes the
//! sheets.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::DbError;

/// An item whose thumbnails are missing or were made from an older file.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TrickplayCandidate {
    pub id: Uuid,
    pub title: String,
    pub season_number: Option<i32>,
    pub episode_number: Option<i32>,
    pub file_path: String,
    pub duration_seconds: f64,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub color_transfer: Option<String>,
    pub file_size: Option<i64>,
    pub file_modified_at: Option<DateTime<Utc>>,
}

/// Newest first, so a freshly scanned film gets previews before the back
/// catalogue. Only probed movies and episodes with a real video stream
/// (cover-art pictures excluded).
pub async fn trickplay_candidates(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<TrickplayCandidate>, DbError> {
    Ok(sqlx::query_as(
        r#"
        SELECT m.id, m.title, m.season_number, m.episode_number,
               m.file_path, m.duration_seconds, v.width, v.height, v.color_transfer,
               m.file_size, m.file_modified_at
        FROM media_items m
        JOIN LATERAL (
            SELECT s.width, s.height, s.color_transfer
            FROM media_streams s
            WHERE s.media_item_id = m.id AND s.stream_type = 'video'
              AND coalesce(s.codec, '') NOT IN ('mjpeg', 'png', 'bmp', 'gif', 'webp')
            ORDER BY s.stream_index
            LIMIT 1
        ) v ON true
        LEFT JOIN media_trickplay t ON t.item_id = m.id
        WHERE m.item_type IN ('movie', 'episode')
          AND m.probed_at IS NOT NULL AND m.probe_error IS NULL
          AND m.duration_seconds > 0
          AND (t.item_id IS NULL
               OR t.source_size IS DISTINCT FROM m.file_size
               OR t.source_modified_at IS DISTINCT FROM m.file_modified_at)
        ORDER BY m.created_at DESC, m.id
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// What a ready item's sheets contain.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Trickplay {
    pub item_id: Uuid,
    pub interval_seconds: i32,
    pub tile_width: i32,
    pub tile_height: i32,
    pub grid_columns: i32,
    pub grid_rows: i32,
    pub thumbnail_count: i32,
    pub sheet_count: i32,
    pub generated_at: DateTime<Utc>,
}

pub struct TrickplayReady<'a> {
    pub item_id: Uuid,
    pub interval_seconds: i32,
    pub tile_width: i32,
    pub tile_height: i32,
    pub grid_columns: i32,
    pub grid_rows: i32,
    pub thumbnail_count: i32,
    pub sheet_count: i32,
    pub source_size: Option<i64>,
    pub source_modified_at: Option<&'a DateTime<Utc>>,
}

pub async fn save_trickplay_ready(
    pool: &PgPool,
    ready: &TrickplayReady<'_>,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO media_trickplay (item_id, status, interval_seconds, tile_width, tile_height, grid_columns, grid_rows,
                                     thumbnail_count, sheet_count, source_size, source_modified_at, error, generated_at)
        VALUES ($1, 'ready', $2, $3, $4, $5, $6, $7, $8, $9, $10, NULL, now())
        ON CONFLICT (item_id) DO UPDATE SET
            status = 'ready', interval_seconds = $2, tile_width = $3, tile_height = $4, grid_columns = $5,
            grid_rows = $6, thumbnail_count = $7, sheet_count = $8, source_size = $9,
            source_modified_at = $10, error = NULL, generated_at = now()
        "#,
    )
    .bind(ready.item_id)
    .bind(ready.interval_seconds)
    .bind(ready.tile_width)
    .bind(ready.tile_height)
    .bind(ready.grid_columns)
    .bind(ready.grid_rows)
    .bind(ready.thumbnail_count)
    .bind(ready.sheet_count)
    .bind(ready.source_size)
    .bind(ready.source_modified_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Remembers a failure so an unchanged broken file is not retried forever.
pub async fn save_trickplay_failed(
    pool: &PgPool,
    item_id: Uuid,
    error: &str,
    source_size: Option<i64>,
    source_modified_at: Option<&DateTime<Utc>>,
) -> Result<(), DbError> {
    let error: String = error.chars().take(1000).collect();
    sqlx::query(
        r#"
        INSERT INTO media_trickplay (item_id, status, source_size, source_modified_at, error, generated_at)
        VALUES ($1, 'failed', $2, $3, $4, now())
        ON CONFLICT (item_id) DO UPDATE SET
            status = 'failed', interval_seconds = 0, tile_width = 0, tile_height = 0, grid_columns = 0, grid_rows = 0,
            thumbnail_count = 0, sheet_count = 0, source_size = $2, source_modified_at = $3,
            error = $4, generated_at = now()
        "#,
    )
    .bind(item_id)
    .bind(source_size)
    .bind(source_modified_at)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

/// The item's sheets, when they are ready.
pub async fn get_trickplay(pool: &PgPool, item_id: Uuid) -> Result<Option<Trickplay>, DbError> {
    Ok(sqlx::query_as(
        "SELECT item_id, interval_seconds, tile_width, tile_height, grid_columns, grid_rows, thumbnail_count, \
         sheet_count, generated_at FROM media_trickplay WHERE item_id = $1 AND status = 'ready'",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?)
}

/// Forgets an item's thumbnails so the worker makes them again. `false`
/// when the item does not exist.
pub async fn reset_trickplay(pool: &PgPool, item_id: Uuid) -> Result<bool, DbError> {
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM media_items WHERE id = $1")
        .bind(item_id)
        .fetch_optional(pool)
        .await?;
    sqlx::query("DELETE FROM media_trickplay WHERE item_id = $1")
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(exists.is_some())
}

/// Ids with ready sheets (to sweep directories left by deleted items).
pub async fn ready_trickplay_ids(pool: &PgPool) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> =
        sqlx::query_as("SELECT item_id FROM media_trickplay WHERE status = 'ready'")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Items trickplay applies to: probed movies and episodes with a real
/// video stream (the same rule as [`trickplay_candidates`]). `current`:
/// the stored result was made from the file as it is now.
const ELIGIBLE: &str = r#"
    SELECT m.id, m.library_id, t.status,
           (t.item_id IS NOT NULL
            AND t.source_size IS NOT DISTINCT FROM m.file_size
            AND t.source_modified_at IS NOT DISTINCT FROM m.file_modified_at) AS current
    FROM media_items m
    LEFT JOIN media_trickplay t ON t.item_id = m.id
    WHERE m.item_type IN ('movie', 'episode')
      AND m.probed_at IS NOT NULL AND m.probe_error IS NULL
      AND m.duration_seconds > 0
      AND EXISTS (
          SELECT 1 FROM media_streams s
          WHERE s.media_item_id = m.id AND s.stream_type = 'video'
            AND coalesce(s.codec, '') NOT IN ('mjpeg', 'png', 'bmp', 'gif', 'webp'))
"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct TrickplayStats {
    /// Thumbnails made from the current file.
    pub ready: i64,
    /// Generation failed for the current file (retried once it changes).
    pub failed: i64,
    /// Still waiting: no thumbnails yet, or made from an older file.
    pub pending: i64,
}

impl TrickplayStats {
    pub fn total(&self) -> i64 {
        self.ready + self.failed + self.pending
    }
}

/// Progress of one library.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct LibraryTrickplayStats {
    pub library_id: Uuid,
    pub name: String,
    pub total: i64,
    pub ready: i64,
    pub failed: i64,
}

/// Every library with its thumbnail progress, in display order. A
/// library without videos has `total` 0.
pub async fn trickplay_library_stats(pool: &PgPool) -> Result<Vec<LibraryTrickplayStats>, DbError> {
    let sql = format!(
        "WITH eligible AS ({ELIGIBLE})
         SELECT l.id AS library_id, l.name,
                count(e.id) AS total,
                count(*) FILTER (WHERE e.current AND e.status = 'ready') AS ready,
                count(*) FILTER (WHERE e.current AND e.status = 'failed') AS failed
         FROM libraries l
         LEFT JOIN eligible e ON e.library_id = l.id
         GROUP BY l.id
         ORDER BY l.sort_order, l.name"
    );
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_all(pool).await?)
}

/// The whole catalog's progress (sums of [`trickplay_library_stats`]).
pub async fn trickplay_stats(pool: &PgPool) -> Result<TrickplayStats, DbError> {
    let sql = format!(
        "WITH eligible AS ({ELIGIBLE})
         SELECT count(*) FILTER (WHERE current AND status = 'ready'),
                count(*) FILTER (WHERE current AND status = 'failed'),
                count(*) FILTER (WHERE NOT current)
         FROM eligible"
    );
    let (ready, failed, pending): (i64, i64, i64) =
        sqlx::query_as(sqlx::AssertSqlSafe(sql)).fetch_one(pool).await?;
    Ok(TrickplayStats {
        ready,
        failed,
        pending,
    })
}

/// A failed item for the admin list.
#[derive(Debug, Clone, PartialEq, serde::Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TrickplayFailure {
    pub item_id: Uuid,
    pub item_type: String,
    pub title: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub library_name: String,
    pub error: Option<String>,
    pub failed_at: DateTime<Utc>,
}

/// The most recent failures, newest first.
pub async fn trickplay_failures(pool: &PgPool, limit: i64) -> Result<Vec<TrickplayFailure>, DbError> {
    Ok(sqlx::query_as(
        r#"
        SELECT t.item_id, m.item_type, m.title, m.season_number AS season,
               m.episode_number AS episode, l.name AS library_name, t.error,
               t.generated_at AS failed_at
        FROM media_trickplay t
        JOIN media_items m ON m.id = t.item_id
        JOIN libraries l ON l.id = m.library_id
        WHERE t.status = 'failed'
        ORDER BY t.generated_at DESC, t.item_id
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?)
}
