//! Stored ffprobe results (`media_streams` + probe columns on
//! `media_items`). A file is probed once per change — by the scanner, or
//! lazily on first playback — instead of on every playback request.

use chrono::{DateTime, SubsecRound, Utc};
use uuid::Uuid;

use lisek_core::media_probe::{MediaChapter, MediaProbe, MediaStream, StreamKind};

use crate::{DbError, PgPool};

/// Size and modification time of a file when it was probed. A stored probe
/// is valid only while the file on disk still matches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: Option<i64>,
    pub modified: Option<DateTime<Utc>>,
}

impl FileStamp {
    pub fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        Self {
            size: i64::try_from(metadata.len()).ok(),
            // Truncated to timestamptz precision; see `scan::file_timestamp`.
            modified: metadata
                .modified()
                .ok()
                .map(|t| DateTime::<Utc>::from(t).trunc_subsecs(6)),
        }
    }

    /// `true` when both stamps carry values and they differ. An unknown
    /// stamp (pre-0014 row, or an OS without mtime) is trusted.
    pub fn differs_from(&self, other: &FileStamp) -> bool {
        let known = |s: &FileStamp| s.size.is_some() && s.modified.is_some();
        known(self) && known(other) && self != other
    }
}

/// Replaces an item's stored streams and probe facts in one transaction.
/// `stamp`, when given, also records the file facts the probe was taken
/// against (playback-time probes; the scanner has already written them).
pub async fn save_probe(
    pool: &PgPool,
    item_id: Uuid,
    probe: &MediaProbe,
    stamp: Option<FileStamp>,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM media_streams WHERE media_item_id = $1")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;

    let s = &probe.streams;
    let col_i32 = |f: fn(&MediaStream) -> Option<i32>| s.iter().map(f).collect::<Vec<_>>();
    let col_text =
        |f: fn(&MediaStream) -> Option<&str>| s.iter().map(f).collect::<Vec<Option<&str>>>();
    sqlx::query(
        r#"
        INSERT INTO media_streams (
            media_item_id, stream_index, stream_type, codec, profile, language, title,
            is_default, is_forced, width, height, bit_depth, pixel_format, color_transfer,
            color_primaries, color_space, field_order, frame_rate, channels, channel_layout,
            sample_rate, bitrate_kbps, dovi_profile)
        SELECT $1, * FROM UNNEST(
            $2::int[], $3::text[], $4::text[], $5::text[], $6::text[], $7::text[],
            $8::bool[], $9::bool[], $10::int[], $11::int[], $12::int[], $13::text[], $14::text[],
            $15::text[], $16::text[], $17::text[], $18::float8[], $19::int[], $20::text[],
            $21::int[], $22::int[], $23::int[])
        ON CONFLICT (media_item_id, stream_index) DO NOTHING
        "#,
    )
    .bind(item_id)
    .bind(s.iter().map(|x| x.index).collect::<Vec<_>>())
    .bind(s.iter().map(|x| x.kind.as_db_str()).collect::<Vec<_>>())
    .bind(col_text(|x| x.codec.as_deref()))
    .bind(col_text(|x| x.profile.as_deref()))
    .bind(col_text(|x| x.language.as_deref()))
    .bind(col_text(|x| x.title.as_deref()))
    .bind(s.iter().map(|x| x.is_default).collect::<Vec<_>>())
    .bind(s.iter().map(|x| x.is_forced).collect::<Vec<_>>())
    .bind(col_i32(|x| x.width))
    .bind(col_i32(|x| x.height))
    .bind(col_i32(|x| x.bit_depth))
    .bind(col_text(|x| x.pixel_format.as_deref()))
    .bind(col_text(|x| x.color_transfer.as_deref()))
    .bind(col_text(|x| x.color_primaries.as_deref()))
    .bind(col_text(|x| x.color_space.as_deref()))
    .bind(col_text(|x| x.field_order.as_deref()))
    .bind(s.iter().map(|x| x.frame_rate).collect::<Vec<_>>())
    .bind(col_i32(|x| x.channels))
    .bind(col_text(|x| x.channel_layout.as_deref()))
    .bind(col_i32(|x| x.sample_rate))
    .bind(col_i32(|x| x.bitrate_kbps))
    .bind(col_i32(|x| x.dovi_profile))
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        UPDATE media_items SET
            container = $2,
            duration_seconds = $3,
            bitrate_kbps = $4,
            probed_at = now(),
            probe_error = NULL,
            file_size = CASE WHEN $5 THEN $6 ELSE file_size END,
            file_modified_at = CASE WHEN $5 THEN $7 ELSE file_modified_at END,
            -- 0 when ffprobe reported none, so the row no longer looks
            -- like a pre-0015 probe.
            start_time_seconds = COALESCE($8, 0),
            chapters_probed = true
        WHERE id = $1
        "#,
    )
    .bind(item_id)
    .bind(probe.container.as_deref())
    .bind(probe.duration_seconds)
    .bind(probe.bitrate_kbps)
    .bind(stamp.is_some())
    .bind(stamp.and_then(|s| s.size))
    .bind(stamp.and_then(|s| s.modified))
    .bind(probe.start_time_seconds)
    .execute(&mut *tx)
    .await?;

    // Chapters: replaced only when they changed, so the playback-time
    // re-probe of an unchanged file writes nothing here.
    let chapters = &probe.chapters;
    let starts: Vec<f64> = chapters.iter().map(|c| c.start_seconds).collect();
    let ends: Vec<Option<f64>> = chapters.iter().map(|c| c.end_seconds).collect();
    let titles: Vec<Option<&str>> = chapters.iter().map(|c| c.title.as_deref()).collect();
    sqlx::query(
        r#"
        WITH wanted AS (
            SELECT (n - 1)::int AS chapter_index, start_seconds, end_seconds, title
            FROM UNNEST($2::float8[], $3::float8[], $4::text[])
                 WITH ORDINALITY AS c(start_seconds, end_seconds, title, n)
        ), removed AS (
            DELETE FROM media_chapters mc
            WHERE mc.media_item_id = $1
              AND NOT EXISTS (SELECT 1 FROM wanted w
                              WHERE w.chapter_index = mc.chapter_index)
        )
        INSERT INTO media_chapters (media_item_id, chapter_index, start_seconds, end_seconds, title)
        SELECT $1, chapter_index, start_seconds, end_seconds, title FROM wanted
        ON CONFLICT (media_item_id, chapter_index) DO UPDATE SET
            start_seconds = EXCLUDED.start_seconds,
            end_seconds = EXCLUDED.end_seconds,
            title = EXCLUDED.title
        WHERE (media_chapters.start_seconds, media_chapters.end_seconds, media_chapters.title)
              IS DISTINCT FROM (EXCLUDED.start_seconds, EXCLUDED.end_seconds, EXCLUDED.title)
        "#,
    )
    .bind(item_id)
    .bind(&starts)
    .bind(&ends)
    .bind(&titles)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// What the detail page reads from an item row: its whole `metadata`
/// (provider data: tagline, genres, ids ...), the probed duration and
/// whether chapters were read from the file yet.
pub async fn details_source(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<(serde_json::Value, Option<f64>, bool)>, DbError> {
    Ok(sqlx::query_as(
        "SELECT metadata, duration_seconds, chapters_probed AND probe_error IS NULL
         FROM media_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?)
}

/// Which of `ids` still name an item (the chapter thumbnail sweep).
pub async fn existing_items(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM media_items WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// An item's chapters in start order (empty when the file has none or was
/// not probed yet).
pub async fn chapters(pool: &PgPool, item_id: Uuid) -> Result<Vec<MediaChapter>, DbError> {
    let rows: Vec<(f64, Option<f64>, Option<String>)> = sqlx::query_as(
        "SELECT start_seconds, end_seconds, title FROM media_chapters
         WHERE media_item_id = $1 ORDER BY chapter_index",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(start_seconds, end_seconds, title)| MediaChapter {
            start_seconds,
            end_seconds,
            title,
        })
        .collect())
}

/// Records that ffprobe could not read this file, so an unchanged broken
/// file is skipped by later scans. A changed file clears the mark (see the
/// scan upsert).
pub async fn save_probe_error(pool: &PgPool, item_id: Uuid, error: &str) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE media_items SET probed_at = now(), probe_error = $2, chapters_probed = true WHERE id = $1",
    )
        .bind(item_id)
        .bind(error)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ProbeFacts {
    container: Option<String>,
    duration_seconds: Option<f64>,
    bitrate_kbps: Option<i32>,
    start_time_seconds: Option<f64>,
    file_size: Option<i64>,
    file_modified_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct StreamRow {
    stream_index: i32,
    stream_type: String,
    codec: Option<String>,
    profile: Option<String>,
    language: Option<String>,
    title: Option<String>,
    is_default: bool,
    is_forced: bool,
    width: Option<i32>,
    height: Option<i32>,
    bit_depth: Option<i32>,
    pixel_format: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    color_space: Option<String>,
    field_order: Option<String>,
    frame_rate: Option<f64>,
    channels: Option<i32>,
    channel_layout: Option<String>,
    sample_rate: Option<i32>,
    bitrate_kbps: Option<i32>,
    dovi_profile: Option<i32>,
}

impl From<StreamRow> for MediaStream {
    fn from(r: StreamRow) -> Self {
        MediaStream {
            index: r.stream_index,
            kind: StreamKind::parse(&r.stream_type),
            codec: r.codec,
            profile: r.profile,
            language: r.language,
            title: r.title,
            is_default: r.is_default,
            is_forced: r.is_forced,
            width: r.width,
            height: r.height,
            bit_depth: r.bit_depth,
            pixel_format: r.pixel_format,
            color_transfer: r.color_transfer,
            color_primaries: r.color_primaries,
            color_space: r.color_space,
            field_order: r.field_order,
            frame_rate: r.frame_rate,
            channels: r.channels,
            channel_layout: r.channel_layout,
            sample_rate: r.sample_rate,
            bitrate_kbps: r.bitrate_kbps,
            dovi_profile: r.dovi_profile,
        }
    }
}

/// The stored probe of an item and the file stamp it was taken against.
/// `None` when the item was never probed successfully (or ffprobe failed
/// on it) — the caller then probes and calls [`save_probe`].
pub async fn stored_probe(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<(MediaProbe, FileStamp)>, DbError> {
    let facts: Option<ProbeFacts> = sqlx::query_as(
        r#"
        SELECT container, duration_seconds, bitrate_kbps, start_time_seconds, file_size,
               file_modified_at
        FROM media_items
        WHERE id = $1 AND probed_at IS NOT NULL AND probe_error IS NULL
        "#,
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    let Some(facts) = facts else {
        return Ok(None);
    };

    let streams: Vec<StreamRow> = sqlx::query_as(
        r#"
        SELECT stream_index, stream_type, codec, profile, language, title, is_default, is_forced,
               width, height, bit_depth, pixel_format, color_transfer, color_primaries,
               color_space, field_order, frame_rate, channels, channel_layout, sample_rate,
               bitrate_kbps, dovi_profile
        FROM media_streams
        WHERE media_item_id = $1
        ORDER BY stream_index
        "#,
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;

    Ok(Some((
        MediaProbe {
            container: facts.container,
            duration_seconds: facts.duration_seconds,
            bitrate_kbps: facts.bitrate_kbps,
            start_time_seconds: facts.start_time_seconds,
            streams: streams.into_iter().map(MediaStream::from).collect(),
            // Playback does not need them; `chapters()` reads them.
            chapters: Vec::new(),
        },
        FileStamp {
            size: facts.file_size,
            modified: facts.file_modified_at,
        },
    )))
}

/// Keyframe timestamps stored by [`save_keyframes`], if the file still
/// matches `stamp`. An unknown stamp on either side is trusted, as for
/// probes.
pub async fn stored_keyframes(
    pool: &PgPool,
    item_id: Uuid,
    stamp: Option<FileStamp>,
) -> Result<Option<Vec<f64>>, DbError> {
    type Row = (Option<i64>, Option<DateTime<Utc>>, Vec<f64>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT file_size, file_modified_at, keyframes FROM media_keyframes WHERE media_item_id = $1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|(size, modified, keyframes)| {
        let stored = FileStamp { size, modified };
        (!stamp.is_some_and(|s| s.differs_from(&stored))).then_some(keyframes)
    }))
}

/// Stores the keyframe timestamps of an item's video, replacing older ones.
pub async fn save_keyframes(
    pool: &PgPool,
    item_id: Uuid,
    stamp: Option<FileStamp>,
    keyframes: &[f64],
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO media_keyframes (media_item_id, file_size, file_modified_at, keyframes)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (media_item_id) DO UPDATE SET
            file_size = EXCLUDED.file_size,
            file_modified_at = EXCLUDED.file_modified_at,
            keyframes = EXCLUDED.keyframes,
            scanned_at = now()
        "#,
    )
    .bind(item_id)
    .bind(stamp.and_then(|s| s.size))
    .bind(stamp.and_then(|s| s.modified))
    .bind(keyframes)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_stamps_are_trusted_known_ones_compared() {
        let now = Utc::now().trunc_subsecs(6);
        let a = FileStamp { size: Some(1), modified: Some(now) };
        let unknown = FileStamp { size: None, modified: None };
        assert!(!a.differs_from(&a));
        assert!(!a.differs_from(&unknown));
        assert!(a.differs_from(&FileStamp { size: Some(2), modified: Some(now) }));
    }
}
