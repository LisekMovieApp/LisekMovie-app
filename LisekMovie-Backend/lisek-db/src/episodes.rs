//! Episode order: what a player offers after an episode ends.

use sqlx::PgPool;
use uuid::Uuid;

use crate::DbError;

/// The episode a player offers after `item_id`.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct NextEpisode {
    pub id: Uuid,
    pub title: String,
    pub season: i32,
    pub episode: i32,
    pub end_episode: Option<i32>,
    pub overview: Option<String>,
    pub duration_seconds: Option<f64>,
}

/// The next episode of the same series: the lowest (season, episode) after
/// the current one's last episode (a double episode `E01-E02` is followed
/// by `E03`). Specials (season 0) are only offered after a special. Another
/// copy of the same episode (a second file, other quality) is skipped.
/// `None` for a movie, an unknown id or the last episode.
pub async fn next_episode(pool: &PgPool, item_id: Uuid) -> Result<Option<NextEpisode>, DbError> {
    Ok(sqlx::query_as(
        r#"
        WITH current AS (
            SELECT series_id, season_number, coalesce(end_episode_number, episode_number) AS last_episode
            FROM episodes WHERE media_item_id = $1
        )
        SELECT e.media_item_id AS id, m.title, e.season_number AS season, e.episode_number AS episode,
               e.end_episode_number AS end_episode, m.overview, m.duration_seconds
        FROM current c
        JOIN episodes e ON e.series_id = c.series_id
        JOIN media_items m ON m.id = e.media_item_id
        WHERE (e.season_number, e.episode_number) > (c.season_number, c.last_episode)
          AND (e.season_number > 0 OR c.season_number = 0)
        ORDER BY e.season_number, e.episode_number, m.created_at, e.media_item_id
        LIMIT 1
        "#,
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?)
}

/// Every episode of the series `item_id` (an episode) belongs to, in
/// season/episode order. Empty for a movie or an unknown id.
pub async fn series_episode_ids(pool: &PgPool, item_id: Uuid) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT e.media_item_id FROM episodes e
         WHERE e.series_id = (SELECT series_id FROM episodes WHERE media_item_id = $1)
         ORDER BY e.season_number, e.episode_number, e.media_item_id",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// The last episode number of a multi-episode file (`S01E01-E02` → 2).
pub async fn end_episode(pool: &PgPool, item_id: Uuid) -> Result<Option<i32>, DbError> {
    let row: Option<(Option<i32>,)> =
        sqlx::query_as("SELECT end_episode_number FROM media_items WHERE id = $1")
            .bind(item_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(end,)| end))
}
