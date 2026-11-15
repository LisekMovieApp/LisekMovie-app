//! Faza 26: own TV channels. Channels, their schedule slots, ad/bumper
//! fillers, and the generated timeline (`tv_playout_items`).
//!
//! The schedule engine (`lisek-tv`) is pure; this module feeds it from the
//! database and stores what it plans. [`rebuild`] does that in one
//! transaction under a lock on the channel's row, so a viewer (or the
//! stream) reading the timeline sees either the old timeline or the new
//! one, never half of it. "What plays at T" is one indexed lookup
//! ([`item_at`]). Everything survives a restart because the timeline and
//! the engine state live in tables.

use std::collections::HashMap;

use chrono::{DateTime, Duration, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgConnection;
use uuid::Uuid;

use lisek_tv::{
    Candidate, EngineState, FillerFile as EngineFile, FillerMode, FillerPreset as EnginePreset, ItemKind, PlayOrder, PlannedItem,
    SlotMode, SlotSpec, StartMode, TailMode,
};

use crate::{DbError, PgPool};

/// How far ahead the timeline is planned.
pub const HORIZON_HOURS: i64 = 72;
/// The builder extends a timeline once less than this much of the horizon is left planned.
const TOP_UP_AFTER_HOURS: i64 = 6;
/// Entries that ended longer ago than this are deleted.
const HISTORY_HOURS: i64 = 24;

// ---------------------------------------------------------------------
// Channels
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct TvChannel {
    pub id: Uuid,
    pub number: String,
    pub name: String,
    pub group_title: Option<String>,
    pub has_logo: bool,
    pub enabled: bool,
    pub position: i32,
    pub timezone: String,
    pub quality: String,
    pub fallback_filler_id: Option<Uuid>,
    pub keep_running: bool,
    pub schedule_version: i32,
    pub created_at: DateTime<Utc>,
}

const CHANNEL_COLUMNS: &str = "id, number, name, group_title, (logo_path IS NOT NULL) AS has_logo, enabled, position, timezone, \
     quality, fallback_filler_id, keep_running, schedule_version, created_at";

/// The editable fields of a channel.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct ChannelInput {
    pub number: String,
    pub name: String,
    pub group_title: Option<String>,
    pub enabled: bool,
    pub timezone: String,
    pub quality: String,
    pub fallback_filler_id: Option<Uuid>,
    pub keep_running: bool,
}

pub async fn list_channels(pool: &PgPool) -> Result<Vec<TvChannel>, DbError> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {CHANNEL_COLUMNS} FROM tv_channels ORDER BY position, sort_number, name")))
        .fetch_all(pool)
        .await?)
}

pub async fn get_channel(pool: &PgPool, id: Uuid) -> Result<Option<TvChannel>, DbError> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {CHANNEL_COLUMNS} FROM tv_channels WHERE id = $1"))).bind(id).fetch_optional(pool).await?)
}

/// Whether another channel already has this number.
pub async fn number_taken(pool: &PgPool, number: &str, except: Option<Uuid>) -> Result<bool, DbError> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tv_channels WHERE number = $1 AND ($2::uuid IS NULL OR id <> $2))")
        .bind(number)
        .bind(except)
        .fetch_one(pool)
        .await?)
}

/// The smallest free whole number from 1, for a channel added without one.
pub async fn next_free_number(pool: &PgPool) -> Result<String, DbError> {
    let taken: Vec<String> = sqlx::query_scalar("SELECT number FROM tv_channels").fetch_all(pool).await?;
    let mut n = 1;
    while taken.iter().any(|t| t == &n.to_string()) {
        n += 1;
    }
    Ok(n.to_string())
}

pub async fn create_channel(pool: &PgPool, input: &ChannelInput) -> Result<TvChannel, DbError> {
    let mut tx = pool.begin().await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO tv_channels (number, name, group_title, enabled, position, timezone, quality, fallback_filler_id, keep_running) \
         VALUES ($1, $2, $3, $4, (SELECT COALESCE(MAX(position) + 1, 0) FROM tv_channels), $5, $6, $7, $8) RETURNING id",
    )
    .bind(&input.number)
    .bind(&input.name)
    .bind(&input.group_title)
    .bind(input.enabled)
    .bind(&input.timezone)
    .bind(&input.quality)
    .bind(input.fallback_filler_id)
    .bind(input.keep_running)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO tv_channel_state (channel_id) VALUES ($1)").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    get_channel(pool, id).await?.ok_or_else(|| DbError::Other("channel vanished".into()))
}

/// Updates a channel. A changed time zone changes what fixed start times
/// mean, so it counts as a schedule edit. `None`: no such channel.
pub async fn update_channel(pool: &PgPool, id: Uuid, input: &ChannelInput) -> Result<Option<TvChannel>, DbError> {
    let updated = sqlx::query(
        "UPDATE tv_channels SET number = $2, name = $3, group_title = $4, enabled = $5, \
             schedule_version = schedule_version + (timezone IS DISTINCT FROM $6)::int, \
             timezone = $6, quality = $7, fallback_filler_id = $8, keep_running = $9, updated_at = now() \
         WHERE id = $1",
    )
    .bind(id)
    .bind(&input.number)
    .bind(&input.name)
    .bind(&input.group_title)
    .bind(input.enabled)
    .bind(&input.timezone)
    .bind(&input.quality)
    .bind(input.fallback_filler_id)
    .bind(input.keep_running)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Ok(None);
    }
    get_channel(pool, id).await
}

/// Deletes a channel with its slots, state and timeline. The logo path
/// (to delete the file) is returned.
pub async fn delete_channel(pool: &PgPool, id: Uuid) -> Result<Option<Option<String>>, DbError> {
    let row: Option<(Option<String>,)> = sqlx::query_as("DELETE FROM tv_channels WHERE id = $1 RETURNING logo_path")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0))
}

/// Stores the new order: `ids` first, in that order.
pub async fn reorder_channels(pool: &PgPool, ids: &[Uuid]) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for (position, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE tv_channels SET position = $2 WHERE id = $1").bind(id).bind(position as i32).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn logo_path(pool: &PgPool, id: Uuid) -> Result<Option<String>, DbError> {
    Ok(sqlx::query_scalar("SELECT logo_path FROM tv_channels WHERE id = $1").bind(id).fetch_optional(pool).await?.flatten())
}

pub async fn set_logo_path(pool: &PgPool, id: Uuid, path: Option<&str>) -> Result<bool, DbError> {
    Ok(sqlx::query("UPDATE tv_channels SET logo_path = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(path)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

// ---------------------------------------------------------------------
// Slots
// ---------------------------------------------------------------------

/// A schedule slot as the admin edits it.
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SlotInput {
    /// Local time of day, `HH:MM`; none: right after the slot before.
    #[serde(default)]
    pub start_time: Option<String>,
    pub start_mode: String,
    pub days_of_week: i16,
    pub source_kind: String,
    pub source_id: Uuid,
    pub play_order: String,
    pub mode: String,
    pub count: Option<i32>,
    pub duration_minutes: Option<i32>,
    pub tail_mode: String,
    pub pre_filler_id: Option<Uuid>,
    pub mid_filler_id: Option<Uuid>,
    pub post_filler_id: Option<Uuid>,
    pub tail_filler_id: Option<Uuid>,
    pub title_override: Option<String>,
    pub in_guide: bool,
}

/// A stored slot, with the name of its source for display.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Slot {
    pub id: Uuid,
    pub position: i32,
    #[serde(flatten)]
    pub slot: SlotInput,
    /// Name of the library, series, film or collection; none if it is gone.
    pub source_label: Option<String>,
}

#[derive(sqlx::FromRow)]
struct SlotRow {
    id: Uuid,
    position: i32,
    start_time: Option<NaiveTime>,
    start_mode: String,
    days_of_week: i16,
    source_kind: String,
    source_id: Uuid,
    play_order: String,
    mode: String,
    count: Option<i32>,
    duration_minutes: Option<i32>,
    tail_mode: String,
    pre_filler_id: Option<Uuid>,
    mid_filler_id: Option<Uuid>,
    post_filler_id: Option<Uuid>,
    tail_filler_id: Option<Uuid>,
    title_override: Option<String>,
    in_guide: bool,
}

const SLOT_COLUMNS: &str = "id, position, start_time, start_mode, days_of_week, source_kind, source_id, play_order, mode, count, \
     duration_minutes, tail_mode, pre_filler_id, mid_filler_id, post_filler_id, tail_filler_id, title_override, in_guide";

fn slot_from_row(r: SlotRow) -> (Uuid, i32, SlotInput) {
    (
        r.id,
        r.position,
        SlotInput {
            start_time: r.start_time.map(|t| t.format("%H:%M").to_string()),
            start_mode: r.start_mode,
            days_of_week: r.days_of_week,
            source_kind: r.source_kind,
            source_id: r.source_id,
            play_order: r.play_order,
            mode: r.mode,
            count: r.count,
            duration_minutes: r.duration_minutes,
            tail_mode: r.tail_mode,
            pre_filler_id: r.pre_filler_id,
            mid_filler_id: r.mid_filler_id,
            post_filler_id: r.post_filler_id,
            tail_filler_id: r.tail_filler_id,
            title_override: r.title_override,
            in_guide: r.in_guide,
        },
    )
}

async fn slots_of(conn: &mut PgConnection, channel_id: Uuid) -> Result<Vec<(Uuid, i32, SlotInput)>, DbError> {
    let rows: Vec<SlotRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {SLOT_COLUMNS} FROM tv_slots WHERE channel_id = $1 ORDER BY position")))
        .bind(channel_id)
        .fetch_all(conn)
        .await?;
    Ok(rows.into_iter().map(slot_from_row).collect())
}

/// The slots of a channel in order, with their source names.
pub async fn list_slots(pool: &PgPool, channel_id: Uuid) -> Result<Vec<Slot>, DbError> {
    let mut conn = pool.acquire().await?;
    let mut out = Vec::new();
    for (id, position, slot) in slots_of(&mut conn, channel_id).await? {
        let source_label = source_label(&mut conn, &slot.source_kind, slot.source_id).await?;
        out.push(Slot { id, position, slot, source_label });
    }
    Ok(out)
}

fn parse_start(value: &Option<String>) -> Option<NaiveTime> {
    value.as_deref().and_then(|v| NaiveTime::parse_from_str(v, "%H:%M").ok())
}

async fn insert_slots(tx: &mut PgConnection, channel_id: Uuid, slots: &[SlotInput]) -> Result<(), DbError> {
    for (position, s) in slots.iter().enumerate() {
        sqlx::query(
            "INSERT INTO tv_slots (channel_id, position, start_time, start_mode, days_of_week, source_kind, source_id, play_order, mode, \
                 count, duration_minutes, tail_mode, pre_filler_id, mid_filler_id, post_filler_id, tail_filler_id, title_override, in_guide) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18)",
        )
        .bind(channel_id)
        .bind(position as i32)
        .bind(parse_start(&s.start_time))
        .bind(&s.start_mode)
        .bind(s.days_of_week)
        .bind(&s.source_kind)
        .bind(s.source_id)
        .bind(&s.play_order)
        .bind(&s.mode)
        .bind(s.count)
        .bind(s.duration_minutes)
        .bind(&s.tail_mode)
        .bind(s.pre_filler_id)
        .bind(s.mid_filler_id)
        .bind(s.post_filler_id)
        .bind(s.tail_filler_id)
        .bind(s.title_override.as_deref().map(str::trim).filter(|t| !t.is_empty()))
        .bind(s.in_guide)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE tv_channels SET schedule_version = schedule_version + 1, updated_at = now() WHERE id = $1")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE tv_channel_state SET dirty = true WHERE channel_id = $1").bind(channel_id).execute(&mut *tx).await?;
    Ok(())
}

/// Replaces a channel's whole schedule. `false`: no such channel.
pub async fn replace_slots(pool: &PgPool, channel_id: Uuid, slots: &[SlotInput]) -> Result<bool, DbError> {
    let mut tx = pool.begin().await?;
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM tv_channels WHERE id = $1 FOR UPDATE").bind(channel_id).fetch_optional(&mut *tx).await?;
    if exists.is_none() {
        return Ok(false);
    }
    sqlx::query("DELETE FROM tv_slots WHERE channel_id = $1").bind(channel_id).execute(&mut *tx).await?;
    insert_slots(&mut tx, channel_id, slots).await?;
    tx.commit().await?;
    Ok(true)
}

/// Copies the schedule of `from` onto `to`, replacing what `to` had.
/// `false`: one of the channels does not exist.
pub async fn copy_schedule(pool: &PgPool, from: Uuid, to: Uuid) -> Result<bool, DbError> {
    let mut tx = pool.begin().await?;
    let both: i64 = sqlx::query_scalar("SELECT count(*) FROM tv_channels WHERE id = ANY($1)").bind(vec![from, to]).fetch_one(&mut *tx).await?;
    if both < 2 && from != to {
        return Ok(false);
    }
    let slots: Vec<SlotInput> = slots_of(&mut tx, from).await?.into_iter().map(|(_, _, s)| s).collect();
    sqlx::query("SELECT id FROM tv_channels WHERE id = $1 FOR UPDATE").bind(to).fetch_one(&mut *tx).await?;
    sqlx::query("DELETE FROM tv_slots WHERE channel_id = $1").bind(to).execute(&mut *tx).await?;
    insert_slots(&mut tx, to, &slots).await?;
    tx.commit().await?;
    Ok(true)
}

// ---------------------------------------------------------------------
// Sources: what a slot can play
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct SourceHit {
    /// `library`, `series`, `movie` or `collection`.
    pub kind: String,
    pub id: Uuid,
    pub title: String,
    /// "12 odcinków", the year, the library type...
    pub detail: Option<String>,
}

/// Libraries, series, films and collections matching `query` (all of the
/// first few when empty).
pub async fn search_sources(pool: &PgPool, query: &str, limit: i64) -> Result<Vec<SourceHit>, DbError> {
    let like = format!("%{}%", query.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    Ok(sqlx::query_as(
        "SELECT * FROM ( \
            SELECT 'library'::text AS kind, id, name AS title, library_type AS detail, 0 AS rank FROM libraries \
                WHERE library_type IN ('movies', 'tv_shows') AND name ILIKE $1 \
            UNION ALL \
            SELECT 'series', id, title, episode_count::text, 1 FROM series WHERE title ILIKE $1 \
            UNION ALL \
            SELECT 'collection', id, name, NULL, 2 FROM collections WHERE name ILIKE $1 \
            UNION ALL \
            SELECT 'movie', id, title, year::text, 3 FROM media_items WHERE item_type = 'movie' AND title ILIKE $1 \
         ) hits ORDER BY rank, title LIMIT $2",
    )
    .bind(like)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

async fn source_label(conn: &mut PgConnection, kind: &str, id: Uuid) -> Result<Option<String>, DbError> {
    let sql = match kind {
        "library" => "SELECT name FROM libraries WHERE id = $1",
        "series" => "SELECT title FROM series WHERE id = $1",
        "movie" => "SELECT title FROM media_items WHERE id = $1",
        "collection" => "SELECT name FROM collections WHERE id = $1",
        _ => return Ok(None),
    };
    Ok(sqlx::query_scalar(sql).bind(id).fetch_optional(conn).await?)
}

type CandidateRow = (Uuid, Option<f64>, Option<i32>, Option<i32>, Option<i32>, String, Option<String>);

const CANDIDATE_SELECT: &str = "SELECT m.id, m.duration_seconds, m.season_number, m.episode_number, m.year, m.title, \
         m.metadata->'provider_data'->'episode'->>'title' \
     FROM media_items m";

fn candidate_from_row(r: CandidateRow) -> Option<Candidate> {
    let seconds = r.1.filter(|s| s.is_finite() && *s > 1.0)?;
    let subtitle = match (r.2, r.3) {
        (Some(season), Some(episode)) => {
            let code = format!("S{season:02}E{episode:02}");
            Some(match r.6.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
                Some(name) => format!("{code} {name}"),
                None => code,
            })
        }
        _ => None,
    };
    Some(Candidate {
        id: r.0,
        duration_ms: (seconds * 1000.0).round() as i64,
        season: r.2,
        episode: r.3,
        year: r.4,
        title: r.5,
        subtitle,
    })
}

/// The films and episodes a slot source stands for, that have a known
/// duration. A source that is gone gives an empty list.
pub async fn candidates(conn: &mut PgConnection, kind: &str, id: Uuid) -> Result<Vec<Candidate>, DbError> {
    let sql = match kind {
        "library" => format!("{CANDIDATE_SELECT} WHERE m.library_id = $1 AND m.item_type IN ('movie', 'episode')"),
        "series" => format!("{CANDIDATE_SELECT} JOIN episodes e ON e.media_item_id = m.id WHERE e.series_id = $1"),
        "movie" => format!("{CANDIDATE_SELECT} WHERE m.id = $1"),
        "collection" => format!("{CANDIDATE_SELECT} JOIN collection_items ci ON ci.media_item_id = m.id WHERE ci.collection_id = $1"),
        _ => return Ok(Vec::new()),
    };
    let rows: Vec<CandidateRow> = sqlx::query_as(sqlx::AssertSqlSafe(sql)).bind(id).fetch_all(conn).await?;
    Ok(rows.into_iter().filter_map(candidate_from_row).collect())
}

// ---------------------------------------------------------------------
// Fillers
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct FillerFolder {
    pub id: Uuid,
    pub path: String,
    pub enabled: bool,
    pub scanned_at: Option<DateTime<Utc>>,
    pub file_count: i32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct FillerFileRow {
    pub id: Uuid,
    pub folder_id: Uuid,
    pub path: String,
    pub duration_ms: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct FillerPresetRow {
    #[serde(default)]
    pub id: Uuid,
    pub name: String,
    /// `pre`, `mid`, `post`, `tail` or `fallback`.
    pub kind: String,
    /// `count`, `duration` or `pad`.
    pub mode: String,
    pub count: Option<i32>,
    pub duration_seconds: Option<i32>,
    pub pad_minutes: Option<i32>,
    #[serde(default)]
    pub folder_ids: Vec<Uuid>,
}

pub async fn list_folders(pool: &PgPool) -> Result<Vec<FillerFolder>, DbError> {
    Ok(sqlx::query_as("SELECT id, path, enabled, scanned_at, file_count, last_error FROM tv_filler_folders ORDER BY path").fetch_all(pool).await?)
}

pub async fn get_folder(pool: &PgPool, id: Uuid) -> Result<Option<FillerFolder>, DbError> {
    Ok(sqlx::query_as("SELECT id, path, enabled, scanned_at, file_count, last_error FROM tv_filler_folders WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

/// Adds a folder. `None`: that path is already there.
pub async fn add_folder(pool: &PgPool, path: &str) -> Result<Option<FillerFolder>, DbError> {
    Ok(sqlx::query_as(
        "INSERT INTO tv_filler_folders (path) VALUES ($1) ON CONFLICT (path) DO NOTHING \
         RETURNING id, path, enabled, scanned_at, file_count, last_error",
    )
    .bind(path)
    .fetch_optional(pool)
    .await?)
}

pub async fn set_folder_enabled(pool: &PgPool, id: Uuid, enabled: bool) -> Result<bool, DbError> {
    let done = sqlx::query("UPDATE tv_filler_folders SET enabled = $2 WHERE id = $1").bind(id).bind(enabled).execute(pool).await?.rows_affected() > 0;
    if done {
        mark_all_dirty(pool).await?;
    }
    Ok(done)
}

/// Removes a folder, its files, and it from every preset.
pub async fn delete_folder(pool: &PgPool, id: Uuid) -> Result<bool, DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE tv_filler_presets SET folder_ids = array_remove(folder_ids, $1) WHERE $1 = ANY(folder_ids)").bind(id).execute(&mut *tx).await?;
    let done = sqlx::query("DELETE FROM tv_filler_folders WHERE id = $1").bind(id).execute(&mut *tx).await?.rows_affected() > 0;
    tx.commit().await?;
    if done {
        mark_all_dirty(pool).await?;
    }
    Ok(done)
}

/// Replaces a folder's files with what a scan found (path, duration in ms).
/// Files keep their id when their path stays, so a rescan does not break
/// timeline entries that point at them.
pub async fn replace_folder_files(pool: &PgPool, folder_id: Uuid, files: &[(String, i32)], error: Option<&str>) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    let paths: Vec<String> = files.iter().map(|f| f.0.clone()).collect();
    let durations: Vec<i32> = files.iter().map(|f| f.1).collect();
    sqlx::query(
        "INSERT INTO tv_filler_files (folder_id, path, duration_ms) SELECT $1, * FROM UNNEST($2::text[], $3::int[]) \
         ON CONFLICT (folder_id, path) DO UPDATE SET duration_ms = EXCLUDED.duration_ms",
    )
    .bind(folder_id)
    .bind(&paths)
    .bind(&durations)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM tv_filler_files WHERE folder_id = $1 AND NOT (path = ANY($2))").bind(folder_id).bind(&paths).execute(&mut *tx).await?;
    sqlx::query("UPDATE tv_filler_folders SET scanned_at = now(), file_count = $2, last_error = $3 WHERE id = $1")
        .bind(folder_id)
        .bind(files.len() as i32)
        .bind(error)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    mark_all_dirty(pool).await?;
    Ok(())
}

pub async fn folder_files(pool: &PgPool, folder_id: Uuid) -> Result<Vec<FillerFileRow>, DbError> {
    Ok(sqlx::query_as("SELECT id, folder_id, path, duration_ms FROM tv_filler_files WHERE folder_id = $1 ORDER BY path").bind(folder_id).fetch_all(pool).await?)
}

pub async fn list_presets(pool: &PgPool) -> Result<Vec<FillerPresetRow>, DbError> {
    Ok(sqlx::query_as("SELECT id, name, kind, mode, count, duration_seconds, pad_minutes, folder_ids FROM tv_filler_presets ORDER BY name")
        .fetch_all(pool)
        .await?)
}

/// Creates a preset (`id` nil) or updates one. `None`: no such preset, or
/// the name is taken by another.
pub async fn save_preset(pool: &PgPool, preset: &FillerPresetRow) -> Result<Option<FillerPresetRow>, DbError> {
    let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tv_filler_presets WHERE lower(name) = lower($1) AND id <> $2)")
        .bind(&preset.name)
        .bind(preset.id)
        .fetch_one(pool)
        .await?;
    if taken {
        return Ok(None);
    }
    let row = if preset.id.is_nil() {
        sqlx::query_as(
            "INSERT INTO tv_filler_presets (name, kind, mode, count, duration_seconds, pad_minutes, folder_ids) VALUES ($1,$2,$3,$4,$5,$6,$7) \
             RETURNING id, name, kind, mode, count, duration_seconds, pad_minutes, folder_ids",
        )
        .bind(&preset.name)
        .bind(&preset.kind)
        .bind(&preset.mode)
        .bind(preset.count)
        .bind(preset.duration_seconds)
        .bind(preset.pad_minutes)
        .bind(&preset.folder_ids)
        .fetch_one(pool)
        .await
        .map(Some)?
    } else {
        sqlx::query_as(
            "UPDATE tv_filler_presets SET name=$2, kind=$3, mode=$4, count=$5, duration_seconds=$6, pad_minutes=$7, folder_ids=$8 WHERE id=$1 \
             RETURNING id, name, kind, mode, count, duration_seconds, pad_minutes, folder_ids",
        )
        .bind(preset.id)
        .bind(&preset.name)
        .bind(&preset.kind)
        .bind(&preset.mode)
        .bind(preset.count)
        .bind(preset.duration_seconds)
        .bind(preset.pad_minutes)
        .bind(&preset.folder_ids)
        .fetch_optional(pool)
        .await?
    };
    if row.is_some() {
        mark_all_dirty(pool).await?;
    }
    Ok(row)
}

pub async fn delete_preset(pool: &PgPool, id: Uuid) -> Result<bool, DbError> {
    let done = sqlx::query("DELETE FROM tv_filler_presets WHERE id = $1").bind(id).execute(pool).await?.rows_affected() > 0;
    if done {
        mark_all_dirty(pool).await?;
    }
    Ok(done)
}

/// The presets the engine plans with: each with the files of its enabled folders.
async fn engine_fillers(conn: &mut PgConnection) -> Result<HashMap<Uuid, EnginePreset>, DbError> {
    #[allow(clippy::type_complexity)]
    let presets: Vec<(Uuid, String, Option<i32>, Option<i32>, Option<i32>, Vec<Uuid>)> =
        sqlx::query_as("SELECT id, mode, count, duration_seconds, pad_minutes, folder_ids FROM tv_filler_presets").fetch_all(&mut *conn).await?;
    let files: Vec<(Uuid, Uuid, String, i32)> = sqlx::query_as(
        "SELECT f.id, f.folder_id, f.path, f.duration_ms FROM tv_filler_files f JOIN tv_filler_folders d ON d.id = f.folder_id WHERE d.enabled \
         ORDER BY f.path",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut out = HashMap::new();
    for (id, mode, count, duration, pad, folders) in presets {
        let Some(mode) = FillerMode::from_db(&mode) else { continue };
        let files = files
            .iter()
            .filter(|f| folders.contains(&f.1))
            .map(|f| EngineFile {
                id: f.0,
                duration_ms: i64::from(f.3),
                title: std::path::Path::new(&f.2).file_stem().map_or_else(|| f.2.clone(), |s| s.to_string_lossy().into_owned()),
            })
            .collect();
        out.insert(
            id,
            EnginePreset {
                id,
                mode,
                count: count.unwrap_or(1).max(1) as u32,
                duration_ms: i64::from(duration.unwrap_or(30)) * 1000,
                pad_minutes: pad.unwrap_or(30).max(1) as u32,
                files,
            },
        );
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// The timeline
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RebuildMode {
    /// Plan further ahead; leave everything planned as it is.
    Continue,
    /// A schedule edit: keep what plays now, plan the rest again, keep each
    /// slot's place in its list.
    Refresh,
    /// Like refresh, and every slot starts its list over.
    Reset,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct RebuildResult {
    pub planned: usize,
    pub built_until: DateTime<Utc>,
    pub warnings: Vec<String>,
}

fn engine_slots(rows: Vec<(Uuid, i32, SlotInput)>, candidates: Vec<Vec<Candidate>>) -> Vec<SlotSpec> {
    rows.into_iter()
        .zip(candidates)
        .map(|((id, _, s), candidates)| SlotSpec {
            id,
            start_time: parse_start(&s.start_time),
            start_mode: StartMode::from_db(&s.start_mode).unwrap_or(StartMode::Strict),
            days_of_week: (s.days_of_week & 127) as u8,
            play_order: PlayOrder::from_db(&s.play_order).unwrap_or(PlayOrder::Chronological),
            mode: SlotMode::from_db(&s.mode).unwrap_or(SlotMode::One),
            count: s.count.unwrap_or(1).max(1) as u32,
            duration_minutes: s.duration_minutes.unwrap_or(30).max(1) as u32,
            tail_mode: TailMode::from_db(&s.tail_mode).unwrap_or(TailMode::Offline),
            pre_filler: s.pre_filler_id,
            post_filler: s.post_filler_id,
            tail_filler: s.tail_filler_id,
            title_override: s.title_override,
            in_guide: s.in_guide,
            candidates,
        })
        .collect()
}

/// Plans (more of) a channel's timeline. All of it happens in one
/// transaction holding a lock on the channel row, so concurrent rebuilds
/// queue up and readers never see a partial timeline. `None`: no such channel.
pub async fn rebuild(pool: &PgPool, channel_id: Uuid, mode: RebuildMode, now: DateTime<Utc>) -> Result<Option<RebuildResult>, DbError> {
    let mut tx = pool.begin().await?;
    let channel: Option<(String, i64, i32)> =
        sqlx::query_as("SELECT timezone, seed, schedule_version FROM tv_channels WHERE id = $1 FOR UPDATE").bind(channel_id).fetch_optional(&mut *tx).await?;
    let Some((zone, seed, version)) = channel else { return Ok(None) };
    let tz = lisek_tv::parse_zone(&zone).unwrap_or(lisek_tv::Tz::UTC);

    let state_json: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT state FROM tv_channel_state WHERE channel_id = $1").bind(channel_id).fetch_optional(&mut *tx).await?;
    let mut state: EngineState = state_json.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();

    let rows = slots_of(&mut tx, channel_id).await?;
    let mut candidate_lists = Vec::with_capacity(rows.len());
    for (_, _, slot) in &rows {
        candidate_lists.push(candidates(&mut tx, &slot.source_kind, slot.source_id).await?);
    }
    let slots = engine_slots(rows, candidate_lists);
    let fillers = engine_fillers(&mut tx).await?;

    let last_end: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT max(ends_at) FROM tv_playout_items WHERE channel_id = $1").bind(channel_id).fetch_one(&mut *tx).await?;
    let max_group: Option<i32> = sqlx::query_scalar("SELECT max(guide_group) FROM tv_playout_items WHERE channel_id = $1").bind(channel_id).fetch_one(&mut *tx).await?;

    let from = match mode {
        RebuildMode::Continue => last_end.map_or(now, |end| end.max(now)),
        RebuildMode::Refresh | RebuildMode::Reset => {
            // What plays now stays; the plan starts where it ends.
            let current_end: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT ends_at FROM tv_playout_items WHERE channel_id = $1 AND starts_at <= $2 AND ends_at > $2")
                .bind(channel_id)
                .bind(now)
                .fetch_optional(&mut *tx)
                .await?;
            let cut = current_end.unwrap_or(now);
            sqlx::query("DELETE FROM tv_playout_items WHERE channel_id = $1 AND starts_at >= $2").bind(channel_id).bind(cut).execute(&mut *tx).await?;
            if mode == RebuildMode::Reset {
                state = EngineState::default();
            }
            let (cursor, entered) = lisek_tv::restart_state(&slots, tz, cut);
            state.cursor = cursor;
            state.entered = entered;
            state.next_start = Some(cut);
            cut
        }
    };
    state.guide_group = state.guide_group.max(max_group.map_or(0, |g| g + 1));

    let until = now + Duration::hours(HORIZON_HOURS);
    let out = lisek_tv::build(&lisek_tv::BuildInput { channel_seed: seed, tz, slots: &slots, fillers: &fillers, state, from, until });

    insert_items(&mut tx, channel_id, &out.items).await?;
    sqlx::query("DELETE FROM tv_playout_items WHERE channel_id = $1 AND ends_at < $2")
        .bind(channel_id)
        .bind(now - Duration::hours(HISTORY_HOURS))
        .execute(&mut *tx)
        .await?;
    // A next slot that waits beyond the horizon leaves the rest empty anyway.
    let built_until = out.state.next_start.unwrap_or(until).max(until);
    let message = (!out.warnings.is_empty()).then(|| out.warnings.join("; "));
    sqlx::query(
        "INSERT INTO tv_channel_state (channel_id, state, built_until, built_version, last_build_at, last_error, dirty) \
         VALUES ($1, $2, $3, $4, now(), $5, false) \
         ON CONFLICT (channel_id) DO UPDATE SET state = EXCLUDED.state, built_until = EXCLUDED.built_until, \
             built_version = EXCLUDED.built_version, last_build_at = now(), last_error = EXCLUDED.last_error, dirty = false",
    )
    .bind(channel_id)
    .bind(serde_json::to_value(&out.state).map_err(|e| DbError::Other(e.to_string()))?)
    .bind(built_until)
    .bind(version)
    .bind(message)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(RebuildResult { planned: out.items.len(), built_until, warnings: out.warnings }))
}

async fn insert_items(tx: &mut PgConnection, channel_id: Uuid, items: &[PlannedItem]) -> Result<(), DbError> {
    for chunk in items.chunks(2000) {
        sqlx::query(
            "INSERT INTO tv_playout_items (channel_id, starts_at, ends_at, kind, media_item_id, filler_file_id, in_ms, out_ms, title, subtitle, slot_id, guide_group, in_guide) \
             SELECT $1, * FROM UNNEST($2::timestamptz[], $3::timestamptz[], $4::text[], $5::uuid[], $6::uuid[], $7::int[], $8::int[], $9::text[], $10::text[], $11::uuid[], $12::int[], $13::bool[])",
        )
        .bind(channel_id)
        .bind(chunk.iter().map(|i| i.starts_at).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.ends_at).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.kind.as_db().to_string()).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.media_item_id).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.filler_file_id).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.in_ms as i32).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.out_ms as i32).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.title.clone()).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.subtitle.clone()).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| Some(i.slot_id)).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.guide_group).collect::<Vec<_>>())
        .bind(chunk.iter().map(|i| i.in_guide).collect::<Vec<_>>())
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

type DueRow = (Uuid, Option<DateTime<Utc>>, i32, i32, bool, Option<DateTime<Utc>>);
type StatusRow = (Uuid, Option<DateTime<Utc>>, Option<DateTime<Utc>>, Option<String>, bool);

/// What the builder should do for each enabled channel now.
pub async fn builds_due(pool: &PgPool, now: DateTime<Utc>) -> Result<Vec<(Uuid, RebuildMode)>, DbError> {
    let rows: Vec<DueRow> = sqlx::query_as(
        "SELECT c.id, s.built_until, s.built_version, c.schedule_version, COALESCE(s.dirty, true), s.last_build_at \
         FROM tv_channels c LEFT JOIN tv_channel_state s ON s.channel_id = c.id WHERE c.enabled ORDER BY c.position, c.sort_number",
    )
    .fetch_all(pool)
    .await?;
    let top_up_before = now + Duration::hours(HORIZON_HOURS - TOP_UP_AFTER_HOURS);
    Ok(rows
        .into_iter()
        .filter_map(|(id, built_until, built_version, version, dirty, _)| {
            if built_until.is_none() || dirty || built_version != version {
                // A channel never planned starts clean; an edited one keeps its place.
                Some((id, if built_until.is_none() { RebuildMode::Reset } else { RebuildMode::Refresh }))
            } else if built_until.is_some_and(|until| until < top_up_before) {
                Some((id, RebuildMode::Continue))
            } else {
                None
            }
        })
        .collect())
}

/// Flags every channel that has a schedule so the next builder pass refreshes it.
pub async fn mark_all_dirty(pool: &PgPool) -> Result<(), DbError> {
    sqlx::query("UPDATE tv_channel_state SET dirty = true WHERE channel_id IN (SELECT DISTINCT channel_id FROM tv_slots)").execute(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct ChannelStatus {
    pub channel_id: Uuid,
    pub built_until: Option<DateTime<Utc>>,
    pub last_build_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub dirty: bool,
}

/// How many slots each channel has.
pub async fn slot_counts(pool: &PgPool) -> Result<Vec<(Uuid, i64)>, DbError> {
    Ok(sqlx::query_as("SELECT channel_id, COUNT(*) FROM tv_slots GROUP BY channel_id").fetch_all(pool).await?)
}

pub async fn statuses(pool: &PgPool) -> Result<Vec<ChannelStatus>, DbError> {
    let rows: Vec<StatusRow> =
        sqlx::query_as("SELECT channel_id, built_until, last_build_at, last_error, dirty FROM tv_channel_state").fetch_all(pool).await?;
    Ok(rows.into_iter().map(|r| ChannelStatus { channel_id: r.0, built_until: r.1, last_build_at: r.2, last_error: r.3, dirty: r.4 }).collect())
}

/// One timeline entry, with the file it plays.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct PlayoutEntry {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub kind: ItemKind,
    pub title: String,
    pub subtitle: Option<String>,
    pub media_item_id: Option<Uuid>,
    pub in_ms: i32,
    pub out_ms: i32,
    pub guide_group: i32,
    pub in_guide: bool,
    /// The file to play; none when the film or ad is gone.
    #[serde(skip)]
    pub path: Option<String>,
    /// Where the file's own clock starts (seconds), for films that do not start at 0.
    #[serde(skip)]
    pub origin_seconds: f64,
}

type EntryRow = (DateTime<Utc>, DateTime<Utc>, String, String, Option<String>, Option<Uuid>, i32, i32, i32, bool, Option<String>, Option<f64>);

const ENTRY_SELECT: &str = "SELECT i.starts_at, i.ends_at, i.kind, i.title, i.subtitle, i.media_item_id, i.in_ms, i.out_ms, i.guide_group, i.in_guide, \
         COALESCE(m.file_path, f.path), m.start_time_seconds \
     FROM tv_playout_items i \
     LEFT JOIN media_items m ON m.id = i.media_item_id \
     LEFT JOIN tv_filler_files f ON f.id = i.filler_file_id";

fn entry_from_row(r: EntryRow) -> PlayoutEntry {
    PlayoutEntry {
        starts_at: r.0,
        ends_at: r.1,
        kind: ItemKind::from_db(&r.2).unwrap_or(ItemKind::Content),
        title: r.3,
        subtitle: r.4,
        media_item_id: r.5,
        in_ms: r.6,
        out_ms: r.7,
        guide_group: r.8,
        in_guide: r.9,
        path: r.10,
        origin_seconds: r.11.unwrap_or(0.0),
    }
}

/// The entry playing at `at`, if any (none in a gap).
pub async fn item_at(pool: &PgPool, channel_id: Uuid, at: DateTime<Utc>) -> Result<Option<PlayoutEntry>, DbError> {
    let row: Option<EntryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} WHERE i.channel_id = $1 AND i.starts_at <= $2 AND i.ends_at > $2 ORDER BY i.starts_at DESC LIMIT 1"
    )))
    .bind(channel_id)
    .bind(at)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(entry_from_row))
}

/// The first entry starting after `at`.
pub async fn item_after(pool: &PgPool, channel_id: Uuid, at: DateTime<Utc>) -> Result<Option<PlayoutEntry>, DbError> {
    let row: Option<EntryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} WHERE i.channel_id = $1 AND i.starts_at > $2 ORDER BY i.starts_at LIMIT 1"
    )))
    .bind(channel_id)
    .bind(at)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(entry_from_row))
}

/// A random file of the channel's fallback preset (what plays when the
/// timeline has a gap): path and duration in milliseconds.
pub async fn fallback_file(pool: &PgPool, channel_id: Uuid) -> Result<Option<(String, i32)>, DbError> {
    Ok(sqlx::query_as(
        "SELECT f.path, f.duration_ms FROM tv_channels c \
         JOIN tv_filler_presets p ON p.id = c.fallback_filler_id \
         JOIN tv_filler_files f ON f.folder_id = ANY(p.folder_ids) \
         JOIN tv_filler_folders d ON d.id = f.folder_id AND d.enabled \
         WHERE c.id = $1 ORDER BY random() LIMIT 1",
    )
    .bind(channel_id)
    .fetch_optional(pool)
    .await?)
}

/// Entries overlapping `[from, to)`, in order.
pub async fn items_between(pool: &PgPool, channel_id: Uuid, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<PlayoutEntry>, DbError> {
    let rows: Vec<EntryRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} WHERE i.channel_id = $1 AND i.ends_at > $2 AND i.starts_at < $3 ORDER BY i.starts_at"
    )))
    .bind(channel_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(entry_from_row).collect())
}

/// One programme of the guide: a film or episode with its ads.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct Programme {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub title: String,
    pub subtitle: Option<String>,
    pub media_item_id: Option<Uuid>,
    pub description: Option<String>,
}

/// Groups timeline entries into guide programmes: consecutive entries of one
/// guide group that include a content entry shown in the guide.
pub fn programmes_from(entries: &[PlayoutEntry], descriptions: &HashMap<Uuid, String>) -> Vec<Programme> {
    let mut out: Vec<Programme> = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let group = entries[i].guide_group;
        let mut j = i;
        while j < entries.len() && entries[j].guide_group == group {
            j += 1;
        }
        if let Some(content) = entries[i..j].iter().find(|e| e.kind == ItemKind::Content && e.in_guide) {
            out.push(Programme {
                starts_at: entries[i].starts_at,
                ends_at: entries[j - 1].ends_at,
                title: content.title.clone(),
                subtitle: content.subtitle.clone(),
                media_item_id: content.media_item_id,
                description: content.media_item_id.and_then(|id| descriptions.get(&id).cloned()),
            });
        }
        i = j;
    }
    out
}

/// The guide of one channel between two times.
pub async fn guide(pool: &PgPool, channel_id: Uuid, from: DateTime<Utc>, to: DateTime<Utc>) -> Result<Vec<Programme>, DbError> {
    let entries = items_between(pool, channel_id, from, to).await?;
    let ids: Vec<Uuid> = entries.iter().filter_map(|e| e.media_item_id).collect();
    let described: Vec<(Uuid, Option<String>)> = if ids.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as("SELECT id, overview FROM media_items WHERE id = ANY($1)").bind(&ids).fetch_all(pool).await?
    };
    let descriptions = described.into_iter().filter_map(|(id, text)| Some((id, text?))).collect();
    Ok(programmes_from(&entries, &descriptions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(group: i32, kind: ItemKind, minute: i64, len: i64, title: &str) -> PlayoutEntry {
        let start = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap() + Duration::minutes(minute);
        PlayoutEntry {
            starts_at: start,
            ends_at: start + Duration::minutes(len),
            kind,
            title: title.into(),
            subtitle: None,
            media_item_id: None,
            in_ms: 0,
            out_ms: (len * 60_000) as i32,
            guide_group: group,
            in_guide: true,
            path: None,
            origin_seconds: 0.0,
        }
    }

    #[test]
    fn a_programme_is_a_film_with_its_ads() {
        let entries = [
            entry(1, ItemKind::Pre, 0, 2, "Reklama"),
            entry(1, ItemKind::Content, 2, 40, "Film A"),
            entry(1, ItemKind::Post, 42, 3, "Reklama"),
            entry(2, ItemKind::Tail, 45, 15, "Przerwa"),
            entry(3, ItemKind::Content, 60, 30, "Film B"),
        ];
        let programmes = programmes_from(&entries, &HashMap::new());
        assert_eq!(programmes.len(), 2, "a group without content is not a programme");
        assert_eq!((programmes[0].title.as_str(), programmes[0].starts_at, programmes[0].ends_at), ("Film A", entries[0].starts_at, entries[2].ends_at));
        assert_eq!(programmes[1].title, "Film B");
    }

    #[test]
    fn content_hidden_from_the_guide_is_left_out() {
        let mut hidden = entry(1, ItemKind::Content, 0, 30, "Ukryte");
        hidden.in_guide = false;
        assert!(programmes_from(&[hidden], &HashMap::new()).is_empty());
    }

    #[test]
    fn candidates_need_a_known_duration() {
        let id = Uuid::new_v4();
        assert!(candidate_from_row((id, None, None, None, None, "x".into(), None)).is_none());
        assert!(candidate_from_row((id, Some(0.0), None, None, None, "x".into(), None)).is_none());
        let episode = candidate_from_row((id, Some(1500.4), Some(2), Some(5), Some(2019), "Show".into(), Some("Pilot".into()))).unwrap();
        assert_eq!((episode.duration_ms, episode.subtitle.as_deref()), (1_500_400, Some("S02E05 Pilot")));
    }
}
