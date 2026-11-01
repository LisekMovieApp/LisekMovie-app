//! Faza 20: Live TV — channel lists (M3U) and programme guides (XMLTV) of
//! `live_tv` libraries.
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{DbError, PgPool};

/// The library type of a Live TV library.
pub const LIVE_TV: &str = "live_tv";

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LiveTvSource {
    pub library_id: Uuid,
    pub library_name: String,
    pub m3u_url: String,
    pub xmltv_url: Option<String>,
    pub refreshed_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub channel_count: i32,
    pub program_count: i32,
}

type SourceRow = (Uuid, String, String, Option<String>, Option<DateTime<Utc>>, Option<String>, i32, i32);

fn source(row: SourceRow) -> LiveTvSource {
    LiveTvSource {
        library_id: row.0,
        library_name: row.1,
        m3u_url: row.2,
        xmltv_url: row.3,
        refreshed_at: row.4,
        last_error: row.5,
        channel_count: row.6,
        program_count: row.7,
    }
}

const SOURCE_SELECT: &str = "SELECT s.library_id, l.name, s.m3u_url, s.xmltv_url, s.refreshed_at, s.last_error, \
     s.channel_count, s.program_count FROM live_tv_sources s JOIN libraries l ON l.id = s.library_id";

pub async fn set_source(pool: &PgPool, library_id: Uuid, m3u_url: &str, xmltv_url: Option<&str>) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO live_tv_sources (library_id, m3u_url, xmltv_url) VALUES ($1, $2, $3) \
         ON CONFLICT (library_id) DO UPDATE SET m3u_url = EXCLUDED.m3u_url, xmltv_url = EXCLUDED.xmltv_url",
    )
    .bind(library_id)
    .bind(m3u_url)
    .bind(xmltv_url)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_source(pool: &PgPool, library_id: Uuid) -> Result<Option<LiveTvSource>, DbError> {
    let sql = format!("{SOURCE_SELECT} WHERE s.library_id = $1");
    Ok(sqlx::query_as::<_, SourceRow>(sqlx::AssertSqlSafe(sql))
        .bind(library_id)
        .fetch_optional(pool)
        .await?
        .map(source))
}

pub async fn list_sources(pool: &PgPool) -> Result<Vec<LiveTvSource>, DbError> {
    let sql = format!("{SOURCE_SELECT} ORDER BY l.sort_order, l.name");
    Ok(sqlx::query_as::<_, SourceRow>(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(source)
        .collect())
}

/// Sources not refreshed for `hours` (or never).
pub async fn sources_due(pool: &PgPool, hours: i32) -> Result<Vec<Uuid>, DbError> {
    Ok(sqlx::query_scalar(
        "SELECT library_id FROM live_tv_sources \
         WHERE refreshed_at IS NULL OR refreshed_at < now() - make_interval(hours => $1)",
    )
    .bind(hours)
    .fetch_all(pool)
    .await?)
}

/// One channel as parsed from the playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct NewChannel {
    pub key: String,
    pub tvg_id: Option<String>,
    pub name: String,
    pub number: Option<String>,
    pub group_title: Option<String>,
    pub logo_url: Option<String>,
    pub stream_url: String,
}

/// One programme, for the channel at `channel` in the list passed along.
#[derive(Debug, Clone, PartialEq)]
pub struct NewProgram {
    pub channel: usize,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub icon_url: Option<String>,
}

/// Replaces a library's channels and guide in one transaction. Channels
/// keep their ids across refreshes (by key), so open guides and links stay
/// valid. Returns (channels, programmes) stored.
pub async fn replace_catalog(
    pool: &PgPool,
    library_id: Uuid,
    channels: &[NewChannel],
    programs: &[NewProgram],
) -> Result<(usize, usize), DbError> {
    let mut tx = pool.begin().await?;
    let mut ids = Vec::with_capacity(channels.len());
    for (position, channel) in channels.iter().enumerate() {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO live_channels (library_id, channel_key, tvg_id, name, number, group_title, logo_url, stream_url, position) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (library_id, channel_key) DO UPDATE SET tvg_id = EXCLUDED.tvg_id, name = EXCLUDED.name, \
                 number = EXCLUDED.number, group_title = EXCLUDED.group_title, logo_url = EXCLUDED.logo_url, \
                 stream_url = EXCLUDED.stream_url, position = EXCLUDED.position \
             RETURNING id",
        )
        .bind(library_id)
        .bind(&channel.key)
        .bind(&channel.tvg_id)
        .bind(&channel.name)
        .bind(&channel.number)
        .bind(&channel.group_title)
        .bind(&channel.logo_url)
        .bind(&channel.stream_url)
        .bind(position as i32)
        .fetch_one(&mut *tx)
        .await?;
        ids.push(id);
    }
    sqlx::query("DELETE FROM live_channels WHERE library_id = $1 AND NOT (id = ANY($2))")
        .bind(library_id)
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM live_programs WHERE channel_id = ANY($1)")
        .bind(&ids)
        .execute(&mut *tx)
        .await?;
    let mut stored = 0;
    for chunk in programs.chunks(2000) {
        let mut channel_ids = Vec::with_capacity(chunk.len());
        let mut starts = Vec::with_capacity(chunk.len());
        let mut ends = Vec::with_capacity(chunk.len());
        let mut titles = Vec::with_capacity(chunk.len());
        let mut subtitles = Vec::with_capacity(chunk.len());
        let mut descriptions = Vec::with_capacity(chunk.len());
        let mut categories = Vec::with_capacity(chunk.len());
        let mut icons = Vec::with_capacity(chunk.len());
        for program in chunk {
            let Some(id) = ids.get(program.channel) else { continue };
            if program.ends_at <= program.starts_at {
                continue;
            }
            channel_ids.push(*id);
            starts.push(program.starts_at);
            ends.push(program.ends_at);
            titles.push(program.title.clone());
            subtitles.push(program.subtitle.clone());
            descriptions.push(program.description.clone());
            categories.push(program.category.clone());
            icons.push(program.icon_url.clone());
        }
        stored += channel_ids.len();
        sqlx::query(
            "INSERT INTO live_programs (channel_id, starts_at, ends_at, title, subtitle, description, category, icon_url) \
             SELECT * FROM UNNEST($1::uuid[], $2::timestamptz[], $3::timestamptz[], $4::text[], $5::text[], $6::text[], $7::text[], $8::text[])",
        )
        .bind(&channel_ids)
        .bind(&starts)
        .bind(&ends)
        .bind(&titles)
        .bind(&subtitles)
        .bind(&descriptions)
        .bind(&categories)
        .bind(&icons)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "UPDATE live_tv_sources SET refreshed_at = now(), last_error = NULL, channel_count = $2, program_count = $3 \
         WHERE library_id = $1",
    )
    .bind(library_id)
    .bind(ids.len() as i32)
    .bind(stored as i32)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((ids.len(), stored))
}

/// Records a failed refresh; the channels and guide from before stay.
pub async fn record_error(pool: &PgPool, library_id: Uuid, error: &str) -> Result<(), DbError> {
    sqlx::query("UPDATE live_tv_sources SET refreshed_at = now(), last_error = $2 WHERE library_id = $1")
        .bind(library_id)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LiveChannel {
    pub id: Uuid,
    pub library_id: Uuid,
    pub name: String,
    pub number: Option<String>,
    pub group_title: Option<String>,
    /// Whether a logo exists (served by `/api/livetv/channels/{id}/logo`).
    pub has_logo: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct LiveProgram {
    pub id: i64,
    pub channel_id: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
}

/// Channels of the given libraries (`None`: every Live TV library), in
/// library and playlist order.
pub async fn channels(pool: &PgPool, libraries: Option<&[Uuid]>) -> Result<Vec<LiveChannel>, DbError> {
    type Row = (Uuid, Uuid, String, Option<String>, Option<String>, bool);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.id, c.library_id, c.name, c.number, c.group_title, c.logo_url IS NOT NULL \
         FROM live_channels c JOIN libraries l ON l.id = c.library_id \
         WHERE $1::uuid[] IS NULL OR c.library_id = ANY($1) \
         ORDER BY l.sort_order, l.name, c.position",
    )
    .bind(libraries)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LiveChannel { id: r.0, library_id: r.1, name: r.2, number: r.3, group_title: r.4, has_logo: r.5 })
        .collect())
}

/// Programmes overlapping `[from, to)` on the given libraries' channels.
pub async fn programs(
    pool: &PgPool,
    libraries: Option<&[Uuid]>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<LiveProgram>, DbError> {
    type Row = (i64, Uuid, DateTime<Utc>, DateTime<Utc>, String, Option<String>, Option<String>, Option<String>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT p.id, p.channel_id, p.starts_at, p.ends_at, p.title, p.subtitle, p.description, p.category \
         FROM live_programs p JOIN live_channels c ON c.id = p.channel_id \
         WHERE ($1::uuid[] IS NULL OR c.library_id = ANY($1)) AND p.ends_at > $2 AND p.starts_at < $3 \
         ORDER BY p.channel_id, p.starts_at \
         LIMIT 50000",
    )
    .bind(libraries)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LiveProgram {
            id: r.0,
            channel_id: r.1,
            starts_at: r.2,
            ends_at: r.3,
            title: r.4,
            subtitle: r.5,
            description: r.6,
            category: r.7,
        })
        .collect())
}

/// What the server needs to play or show one channel.
pub struct ChannelSource {
    pub id: Uuid,
    pub library_id: Uuid,
    pub name: String,
    pub stream_url: String,
    pub logo_url: Option<String>,
}

pub async fn channel_source(pool: &PgPool, channel_id: Uuid) -> Result<Option<ChannelSource>, DbError> {
    type Row = (Uuid, Uuid, String, String, Option<String>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, library_id, name, stream_url, logo_url FROM live_channels WHERE id = $1",
    )
    .bind(channel_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| ChannelSource { id: r.0, library_id: r.1, name: r.2, stream_url: r.3, logo_url: r.4 }))
}
