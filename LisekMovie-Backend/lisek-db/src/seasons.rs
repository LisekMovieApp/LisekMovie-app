//! Seasons and episodes of a series for the series pages (Faza 10): season
//! cards, a season's episode list with each viewer's watch state, season
//! metadata from the provider, and bulk watched/favourite flags.
//!
//! Every function takes any episode id of the series (the series card's id
//! is its representative episode). Episode data from the provider lives in
//! the episode's `metadata.provider_data.episode` (title, overview,
//! runtime, rating, still); season data in the `seasons` row (0022).

use uuid::Uuid;

use crate::{DbError, PgPool};

/// Where an episode sits in its series.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Placement {
    pub series_id: Uuid,
    pub season: i32,
    pub episode: i32,
    pub end_episode: Option<i32>,
}

pub async fn placement(pool: &PgPool, item_id: Uuid) -> Result<Option<Placement>, DbError> {
    Ok(sqlx::query_as(
        "SELECT series_id, season_number AS season, episode_number AS episode,
                end_episode_number AS end_episode
         FROM episodes WHERE media_item_id = $1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?)
}

/// A season of a series as stored: number and the poster URL on disk.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StoredSeason {
    pub number: i32,
    pub poster_url: Option<String>,
}

/// The seasons of a series and the files of its episodes (enrichment:
/// season posters, legacy artwork in season folders).
pub async fn series_layout(
    pool: &PgPool,
    series_id: Uuid,
) -> Result<(Vec<StoredSeason>, Vec<String>), DbError> {
    let seasons: Vec<StoredSeason> = sqlx::query_as(
        "SELECT season_number AS number, poster_url FROM seasons
         WHERE series_id = $1 ORDER BY season_number",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await?;
    let files: Vec<(String,)> = sqlx::query_as(
        "SELECT m.file_path FROM episodes e JOIN media_items m ON m.id = e.media_item_id
         WHERE e.series_id = $1 ORDER BY e.season_number, e.episode_number",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await?;
    Ok((seasons, files.into_iter().map(|(f,)| f).collect()))
}

/// Provider facts of one season; unchanged values are not rewritten.
pub async fn set_season_metadata(
    pool: &PgPool,
    series_id: Uuid,
    season: i32,
    name: Option<&str>,
    overview: Option<&str>,
    air_year: Option<i32>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE seasons SET name = $3, overview = $4, air_year = $5
         WHERE series_id = $1 AND season_number = $2
           AND (name, overview, air_year) IS DISTINCT FROM ($3, $4, $5)",
    )
    .bind(series_id)
    .bind(season)
    .bind(name)
    .bind(overview)
    .bind(air_year)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_season_poster_url(
    pool: &PgPool,
    series_id: Uuid,
    season: i32,
    url: Option<&str>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE seasons SET poster_url = $3
         WHERE series_id = $1 AND season_number = $2 AND poster_url IS DISTINCT FROM $3",
    )
    .bind(series_id)
    .bind(season)
    .bind(url)
    .execute(pool)
    .await?;
    Ok(())
}

/// One season card of a series page, with the viewer's progress.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SeasonCard {
    pub season: i32,
    pub name: Option<String>,
    pub episode_count: i64,
    pub unwatched_count: i64,
    /// Every episode is a favourite of the viewer.
    pub favorite: bool,
    /// Where "Play" starts: the first episode not watched yet (the first
    /// episode once all are watched).
    pub play_id: Uuid,
    pub has_poster: bool,
    pub air_year: Option<i32>,
}

pub async fn season_cards(
    pool: &PgPool,
    item_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<SeasonCard>, DbError> {
    Ok(sqlx::query_as(
        "SELECT se.season_number AS season, se.name, count(*) AS episode_count,
                count(*) FILTER (WHERE NOT coalesce(s.watched, false)) AS unwatched_count,
                bool_and(coalesce(s.favorite, false)) AS favorite,
                coalesce(
                    (array_agg(e.media_item_id ORDER BY e.episode_number, e.media_item_id)
                        FILTER (WHERE NOT coalesce(s.watched, false)))[1],
                    (array_agg(e.media_item_id ORDER BY e.episode_number, e.media_item_id))[1]
                ) AS play_id,
                se.poster_url IS NOT NULL AS has_poster, se.air_year
         FROM episodes e
         JOIN seasons se ON se.id = e.season_id
         LEFT JOIN user_media_state s ON s.item_id = e.media_item_id AND s.user_id = $2
         WHERE e.series_id = (SELECT series_id FROM episodes WHERE media_item_id = $1)
         GROUP BY se.id
         ORDER BY (se.season_number = 0), se.season_number",
    )
    .bind(item_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// One row of a season page.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct EpisodeRow {
    pub id: Uuid,
    pub episode: i32,
    pub end_episode: Option<i32>,
    /// `provider_data.episode`: title, overview, runtime_minutes, rating,
    /// still_url, air_date (`null` before enrichment).
    pub data: Option<serde_json::Value>,
    pub duration_seconds: Option<f64>,
    pub watched: bool,
    pub favorite: bool,
    pub position_seconds: f64,
}

/// `(series_id, name, overview, air_year, has_poster)`.
type SeasonHead = (Uuid, Option<String>, Option<String>, Option<i32>, bool);

/// A season page: the season's facts and its episodes in order.
#[derive(Debug, Clone, PartialEq)]
pub struct SeasonPage {
    pub series_id: Uuid,
    pub season: i32,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub air_year: Option<i32>,
    pub has_poster: bool,
    pub episodes: Vec<EpisodeRow>,
}

pub async fn season_page(
    pool: &PgPool,
    item_id: Uuid,
    season: i32,
    user_id: Uuid,
) -> Result<Option<SeasonPage>, DbError> {
    let head: Option<SeasonHead> = sqlx::query_as(
        "SELECT se.series_id, se.name, se.overview, se.air_year, se.poster_url IS NOT NULL
         FROM seasons se
         WHERE se.series_id = (SELECT series_id FROM episodes WHERE media_item_id = $1)
           AND se.season_number = $2",
    )
    .bind(item_id)
    .bind(season)
    .fetch_optional(pool)
    .await?;
    let Some((series_id, name, overview, air_year, has_poster)) = head else {
        return Ok(None);
    };
    let episodes: Vec<EpisodeRow> = sqlx::query_as(
        "SELECT e.media_item_id AS id, e.episode_number AS episode,
                e.end_episode_number AS end_episode,
                m.metadata->'provider_data'->'episode' AS data, m.duration_seconds,
                coalesce(s.watched, false) AS watched, coalesce(s.favorite, false) AS favorite,
                coalesce(s.position_seconds, 0) AS position_seconds
         FROM episodes e
         JOIN media_items m ON m.id = e.media_item_id
         LEFT JOIN user_media_state s ON s.item_id = e.media_item_id AND s.user_id = $3
         WHERE e.series_id = $1 AND e.season_number = $2
         ORDER BY e.episode_number, m.created_at, e.media_item_id",
    )
    .bind(series_id)
    .bind(season)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(Some(SeasonPage {
        series_id,
        season,
        name,
        overview,
        air_year,
        has_poster,
        episodes,
    }))
}

/// Sets watched and/or favourite on every episode of one season for one
/// viewer (watched also clears the resume point, as for one item).
/// Returns how many episodes it covered.
pub async fn set_season_flags(
    pool: &PgPool,
    user_id: Uuid,
    item_id: Uuid,
    season: i32,
    watched: Option<bool>,
    favorite: Option<bool>,
) -> Result<u64, DbError> {
    let done = sqlx::query(
        "INSERT INTO user_media_state (user_id, item_id, watched, favorite)
         SELECT $1, e.media_item_id, coalesce($4, false), coalesce($5, false)
         FROM episodes e
         WHERE e.series_id = (SELECT series_id FROM episodes WHERE media_item_id = $2)
           AND e.season_number = $3
         ON CONFLICT (user_id, item_id) DO UPDATE SET
             watched = coalesce($4, user_media_state.watched),
             position_seconds = CASE WHEN $4 IS NOT NULL THEN 0 ELSE user_media_state.position_seconds END,
             favorite = coalesce($5, user_media_state.favorite),
             updated_at = now()",
    )
    .bind(user_id)
    .bind(item_id)
    .bind(season)
    .bind(watched)
    .bind(favorite)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// The series of `item_id` and one of its episode files (a season poster
/// is looked up next to the series folder).
pub async fn season_poster_source(
    pool: &PgPool,
    item_id: Uuid,
    season: i32,
) -> Result<Option<(Uuid, String)>, DbError> {
    Ok(sqlx::query_as(
        "SELECT e.series_id, m.file_path FROM episodes e JOIN media_items m ON m.id = e.media_item_id
         WHERE e.series_id = (SELECT series_id FROM episodes WHERE media_item_id = $1)
           AND e.season_number = $2
         ORDER BY e.episode_number LIMIT 1",
    )
    .bind(item_id)
    .bind(season)
    .fetch_optional(pool)
    .await?)
}

/// The still URL an episode's provider data names now (to skip
/// downloading it again).
pub async fn episode_still_url(pool: &PgPool, item_id: Uuid) -> Result<Option<String>, DbError> {
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT metadata->'provider_data'->'episode'->>'still_url' FROM media_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(url,)| url))
}
