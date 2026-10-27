//! Watch statistics (Faza 13): totals, rankings and drop-off over the
//! playback sessions of a time range, for the admin dashboard and the bot
//! summary (`GET /api/stats/summary`).
//!
//! Every query starts from the same filtered set `s` (the [`BASE`] CTE):
//! sessions started in `[from, to)` that played at all, narrowed by user,
//! platform, device type, content type and title. The range comes first
//! and is served by `playback_sessions_started`, so a week of a server with
//! years of history reads only that week's rows; the aggregates then run
//! over that slice. The dashboard's queries are independent and run in
//! parallel on separate connections.
//!
//! Counting rules:
//! - viewing time is `watched_seconds` (time actually playing, see
//!   `record_progress`), summed per session;
//! - a "play" is a session with at least [`MIN_PLAY_SECONDS`] of viewing,
//!   so a trailer-length peek or a misclick is not a play;
//! - a session is *completed* once the resume rules called it watched,
//!   *watching* while it reported in the last two minutes, otherwise
//!   *abandoned* — stopped before the end, at `position_seconds`.

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use crate::{DbError, PgPool};

/// Viewing below this does not count as a play (it still counts as time).
pub const MIN_PLAY_SECONDS: f64 = 20.0;

/// Films or series (episodes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Movie,
    Series,
}

impl Content {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "movie" | "movies" => Some(Self::Movie),
            "series" => Some(Self::Series),
            _ => None,
        }
    }
}

/// Which sessions a statistic covers.
#[derive(Debug, Clone)]
pub struct StatsFilter {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub user_id: Option<Uuid>,
    /// `lisek_telemetry::client::Platform::as_str`.
    pub platform: Option<String>,
    /// `lisek_telemetry::client::DeviceType::as_str`.
    pub device_type: Option<String>,
    pub content: Option<Content>,
    /// A film or episode id, or a series id: that title only.
    pub item: Option<Uuid>,
}

impl StatsFilter {
    pub fn range(from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        Self {
            from,
            to,
            user_id: None,
            platform: None,
            device_type: None,
            content: None,
            item: None,
        }
    }
}

/// The filtered sessions. Binds `$1`..`$7` are the filter (see [`bind`]);
/// queries add their own from `$8`.
const BASE: &str = "WITH s AS ( \
    SELECT p.id, p.user_id, p.item_id, p.started_at, p.ended_at, p.last_heartbeat_at, \
           p.watched_seconds, p.position_seconds, p.duration_seconds, p.completed, \
           p.platform, p.device_type, p.browser, m.item_type, e.series_id \
    FROM playback_sessions p \
    JOIN media_items m ON m.id = p.item_id \
    LEFT JOIN episodes e ON e.media_item_id = p.item_id \
    WHERE p.started_at >= $1 AND p.started_at < $2 AND p.watched_seconds > 0 \
      AND ($3::uuid IS NULL OR p.user_id = $3) \
      AND ($4::text IS NULL OR p.platform = $4) \
      AND ($5::text IS NULL OR p.device_type = $5) \
      AND ($6::text IS NULL OR (CASE WHEN m.item_type = 'episode' THEN 'series' ELSE 'movie' END) = $6) \
      AND ($7::uuid IS NULL OR p.item_id = $7 OR e.series_id = $7)) ";

/// "Abandoned": not completed and no longer playing.
const ABANDONED: &str = "NOT s.completed AND (s.ended_at IS NOT NULL OR s.last_heartbeat_at <= now() - interval '2 minutes')";

fn sql(body: &str) -> AssertSqlSafe<String> {
    // Only the fixed fragments above and literal SQL below are formatted
    // in; every value is a bind.
    AssertSqlSafe(format!("{BASE}{body}"))
}

macro_rules! bind_filter {
    ($query:expr, $filter:expr) => {
        $query
            .bind($filter.from)
            .bind($filter.to)
            .bind($filter.user_id)
            .bind($filter.platform.as_deref())
            .bind($filter.device_type.as_deref())
            .bind($filter.content.map(Content::as_str))
            .bind($filter.item)
    };
}

#[derive(Debug, Clone, Default, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Totals {
    pub watched_seconds: f64,
    pub plays: i64,
    /// Different accounts that watched anything.
    pub users: i64,
    /// Different films and episodes.
    pub titles: i64,
    pub completed: i64,
    pub abandoned: i64,
    /// Sessions (plays and shorter ones).
    pub sessions: i64,
}

pub async fn totals(pool: &PgPool, f: &StatsFilter) -> Result<Totals, DbError> {
    let q = sql(&format!(
        "SELECT COALESCE(sum(s.watched_seconds), 0)::float8 AS watched_seconds, \
                count(*) FILTER (WHERE s.watched_seconds >= {MIN_PLAY_SECONDS}) AS plays, \
                count(DISTINCT s.user_id) AS users, count(DISTINCT s.item_id) AS titles, \
                count(*) FILTER (WHERE s.completed) AS completed, \
                count(*) FILTER (WHERE {ABANDONED}) AS abandoned, count(*) AS sessions FROM s"
    ));
    Ok(bind_filter!(sqlx::query_as::<_, Totals>(q), f)
        .fetch_one(pool)
        .await?)
}

/// Viewing per calendar day in `timezone` (an IANA name PostgreSQL knows).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct DayStats {
    pub day: NaiveDate,
    pub watched_seconds: f64,
    pub plays: i64,
    pub users: i64,
}

pub async fn by_day(
    pool: &PgPool,
    f: &StatsFilter,
    timezone: &str,
) -> Result<Vec<DayStats>, DbError> {
    let q = sql(&format!(
        "SELECT (s.started_at AT TIME ZONE $8)::date AS day, sum(s.watched_seconds)::float8 AS watched_seconds, \
                count(*) FILTER (WHERE s.watched_seconds >= {MIN_PLAY_SECONDS}) AS plays, \
                count(DISTINCT s.user_id) AS users \
         FROM s GROUP BY 1 ORDER BY 1"
    ));
    Ok(bind_filter!(sqlx::query_as::<_, DayStats>(q), f)
        .bind(timezone)
        .fetch_all(pool)
        .await?)
}

/// Viewing per hour of the day (0–23) in `timezone`: when people watch.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct HourStats {
    pub hour: i32,
    pub watched_seconds: f64,
    pub sessions: i64,
}

pub async fn by_hour(
    pool: &PgPool,
    f: &StatsFilter,
    timezone: &str,
) -> Result<Vec<HourStats>, DbError> {
    let q = sql(
        "SELECT extract(hour FROM s.started_at AT TIME ZONE $8)::int AS hour, \
                sum(s.watched_seconds)::float8 AS watched_seconds, count(*) AS sessions \
         FROM s GROUP BY 1 ORDER BY 1",
    );
    Ok(bind_filter!(sqlx::query_as::<_, HourStats>(q), f)
        .bind(timezone)
        .fetch_all(pool)
        .await?)
}

/// One slice of a breakdown (platform, device type, content type).
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct Slice {
    pub key: String,
    pub watched_seconds: f64,
    pub plays: i64,
    pub users: i64,
}

async fn slices(pool: &PgPool, f: &StatsFilter, column: &str) -> Result<Vec<Slice>, DbError> {
    let q = sql(&format!(
        "SELECT {column} AS key, sum(s.watched_seconds)::float8 AS watched_seconds, \
                count(*) FILTER (WHERE s.watched_seconds >= {MIN_PLAY_SECONDS}) AS plays, \
                count(DISTINCT s.user_id) AS users \
         FROM s GROUP BY 1 ORDER BY 2 DESC"
    ));
    Ok(bind_filter!(sqlx::query_as::<_, Slice>(q), f)
        .fetch_all(pool)
        .await?)
}

pub async fn by_platform(pool: &PgPool, f: &StatsFilter) -> Result<Vec<Slice>, DbError> {
    slices(pool, f, "s.platform").await
}

pub async fn by_device_type(pool: &PgPool, f: &StatsFilter) -> Result<Vec<Slice>, DbError> {
    slices(pool, f, "s.device_type").await
}

/// `movie` and `series`.
pub async fn by_content(pool: &PgPool, f: &StatsFilter) -> Result<Vec<Slice>, DbError> {
    slices(
        pool,
        f,
        "CASE WHEN s.item_type = 'episode' THEN 'series' ELSE 'movie' END",
    )
    .await
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct UserRank {
    pub user_id: Uuid,
    pub username: String,
    pub watched_seconds: f64,
    pub plays: i64,
    pub titles: i64,
    pub last_watched_at: DateTime<Utc>,
}

pub async fn top_users(
    pool: &PgPool,
    f: &StatsFilter,
    limit: i64,
) -> Result<Vec<UserRank>, DbError> {
    let q = sql(&format!(
        "SELECT u.id AS user_id, u.username, sum(s.watched_seconds)::float8 AS watched_seconds, \
                count(*) FILTER (WHERE s.watched_seconds >= {MIN_PLAY_SECONDS}) AS plays, \
                count(DISTINCT s.item_id) AS titles, max(s.started_at) AS last_watched_at \
         FROM s JOIN users u ON u.id = s.user_id \
         GROUP BY u.id, u.username ORDER BY watched_seconds DESC, u.username LIMIT $8"
    ));
    Ok(bind_filter!(sqlx::query_as::<_, UserRank>(q), f)
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

/// A film or a series in a ranking.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TitleRank {
    /// The film, or the series' representative item: `/media/{id}`.
    pub id: Uuid,
    /// The series itself (series rankings only): filter value.
    pub series_id: Option<Uuid>,
    pub title: String,
    pub year: Option<i32>,
    pub watched_seconds: f64,
    pub plays: i64,
    /// Different accounts that watched it.
    pub users: i64,
    /// Sessions that reached the end.
    pub completed: i64,
}

pub async fn top_movies(
    pool: &PgPool,
    f: &StatsFilter,
    limit: i64,
) -> Result<Vec<TitleRank>, DbError> {
    let q = sql(&format!(
        "SELECT m.id, NULL::uuid AS series_id, m.title, m.year, sum(s.watched_seconds)::float8 AS watched_seconds, \
                count(*) FILTER (WHERE s.watched_seconds >= {MIN_PLAY_SECONDS}) AS plays, \
                count(DISTINCT s.user_id) AS users, count(*) FILTER (WHERE s.completed) AS completed \
         FROM s JOIN media_items m ON m.id = s.item_id WHERE s.item_type = 'movie' \
         GROUP BY m.id, m.title, m.year ORDER BY watched_seconds DESC, plays DESC, m.title LIMIT $8"
    ));
    Ok(bind_filter!(sqlx::query_as::<_, TitleRank>(q), f)
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

pub async fn top_series(
    pool: &PgPool,
    f: &StatsFilter,
    limit: i64,
) -> Result<Vec<TitleRank>, DbError> {
    let q = sql(&format!(
        "SELECT COALESCE(x.representative_item_id, min(s.item_id::text)::uuid) AS id, x.id AS series_id, x.title, \
                min(m.year) AS year, sum(s.watched_seconds)::float8 AS watched_seconds, \
                count(*) FILTER (WHERE s.watched_seconds >= {MIN_PLAY_SECONDS}) AS plays, \
                count(DISTINCT s.user_id) AS users, count(*) FILTER (WHERE s.completed) AS completed \
         FROM s JOIN series x ON x.id = s.series_id JOIN media_items m ON m.id = s.item_id \
         GROUP BY x.id, x.title, x.representative_item_id \
         ORDER BY watched_seconds DESC, plays DESC, x.title LIMIT $8"
    ));
    Ok(bind_filter!(sqlx::query_as::<_, TitleRank>(q), f)
        .bind(limit)
        .fetch_all(pool)
        .await?)
}

/// Abandoned sessions by where they stopped: bucket `n` covers
/// `n*10 % .. (n+1)*10 %` of the title. Always ten buckets.
pub async fn dropoff(pool: &PgPool, f: &StatsFilter) -> Result<Vec<i64>, DbError> {
    let q = sql(&format!(
        "SELECT LEAST(9, GREATEST(0, floor(s.position_seconds / s.duration_seconds * 10)))::int AS bucket, count(*) \
         FROM s WHERE {ABANDONED} AND s.duration_seconds > 0 GROUP BY 1"
    ));
    let rows: Vec<(i32, i64)> = bind_filter!(sqlx::query_as(q), f).fetch_all(pool).await?;
    let mut buckets = vec![0i64; 10];
    for (bucket, count) in rows {
        if let Some(slot) = usize::try_from(bucket)
            .ok()
            .and_then(|b| buckets.get_mut(b))
        {
            *slot = count;
        }
    }
    Ok(buckets)
}

/// One session in the dashboard's list.
#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SessionRow {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub user_id: Uuid,
    pub username: String,
    /// The film, or the episode (`/media/{id}`).
    pub item_id: Uuid,
    /// Film title or series title.
    pub title: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub episode_title: Option<String>,
    pub platform: String,
    pub device_type: String,
    pub browser: Option<String>,
    pub watched_seconds: f64,
    /// Where the viewer was last (stopped, for an abandoned session).
    pub position_seconds: f64,
    /// 0 when the player never reported a length.
    pub duration_seconds: f64,
    /// `completed`, `watching` or `abandoned`.
    pub outcome: String,
}

pub async fn sessions(
    pool: &PgPool,
    f: &StatsFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<SessionRow>, DbError> {
    let q = sql(&format!(
        "SELECT s.id, s.started_at, s.ended_at, s.user_id, u.username, s.item_id, m.title, \
                m.season_number AS season, m.episode_number AS episode, \
                m.metadata->'provider_data'->'episode'->>'title' AS episode_title, \
                s.platform, s.device_type, s.browser, s.watched_seconds, s.position_seconds, s.duration_seconds, \
                CASE WHEN s.completed THEN 'completed' WHEN {ABANDONED} THEN 'abandoned' ELSE 'watching' END AS outcome \
         FROM s JOIN users u ON u.id = s.user_id JOIN media_items m ON m.id = s.item_id \
         ORDER BY s.started_at DESC, s.id LIMIT $8 OFFSET $9"
    ));
    Ok(bind_filter!(sqlx::query_as::<_, SessionRow>(q), f)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?)
}

/// `true` when PostgreSQL knows `name` as a time zone.
pub async fn valid_timezone(pool: &PgPool, name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    sqlx::query("SELECT now() AT TIME ZONE $1")
        .bind(name)
        .execute(pool)
        .await
        .is_ok()
}

/// `dd.mm` of an instant in `timezone`, for report titles ("14.09 - 20.09").
pub async fn day_month(
    pool: &PgPool,
    at: DateTime<Utc>,
    timezone: &str,
) -> Result<String, DbError> {
    let (label,): (String,) = sqlx::query_as("SELECT to_char($1 AT TIME ZONE $2, 'DD.MM')")
        .bind(at)
        .bind(timezone)
        .fetch_one(pool)
        .await?;
    Ok(label)
}

/// `dd.mm.yyyy hh:mi` of an instant in `timezone` (report footers).
pub async fn date_time(
    pool: &PgPool,
    at: DateTime<Utc>,
    timezone: &str,
) -> Result<String, DbError> {
    let (label,): (String,) =
        sqlx::query_as("SELECT to_char($1 AT TIME ZONE $2, 'DD.MM.YYYY HH24:MI')")
            .bind(at)
            .bind(timezone)
            .fetch_one(pool)
            .await?;
    Ok(label)
}

/// The stored hash of the bots' read-only key, if one was created.
pub async fn api_key(pool: &PgPool) -> Result<Option<(String, DateTime<Utc>)>, DbError> {
    Ok(
        sqlx::query_as("SELECT key_hash, created_at FROM stats_api_key WHERE id")
            .fetch_optional(pool)
            .await?,
    )
}

/// Replaces the bots' key (only its hash is stored).
pub async fn set_api_key(pool: &PgPool, key_hash: &str) -> Result<DateTime<Utc>, DbError> {
    let (at,): (DateTime<Utc>,) = sqlx::query_as(
        "INSERT INTO stats_api_key (id, key_hash) VALUES (true, $1) \
         ON CONFLICT (id) DO UPDATE SET key_hash = $1, created_at = now() RETURNING created_at",
    )
    .bind(key_hash)
    .fetch_one(pool)
    .await?;
    Ok(at)
}

pub async fn delete_api_key(pool: &PgPool) -> Result<(), DbError> {
    sqlx::query("DELETE FROM stats_api_key")
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_values() {
        assert_eq!(Content::parse("movies"), Some(Content::Movie));
        assert_eq!(
            Content::parse("series").map(Content::as_str),
            Some("series")
        );
        assert_eq!(Content::parse("episode"), None);
    }
}
