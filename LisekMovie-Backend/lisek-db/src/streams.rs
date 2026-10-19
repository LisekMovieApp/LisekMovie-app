//! Stored ffprobe results (`media_streams` + probe columns on
//! `media_items`). A file is probed once per change — by the scanner, or
//! lazily on first playback — instead of on every playback request.

use chrono::{DateTime, SubsecRound, Utc};
use uuid::Uuid;

use lisek_core::media_probe::{MediaProbe, MediaStream, StreamKind};

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
            sample_rate, bitrate_kbps)
        SELECT $1, * FROM UNNEST(
            $2::int[], $3::text[], $4::text[], $5::text[], $6::text[], $7::text[],
            $8::bool[], $9::bool[], $10::int[], $11::int[], $12::int[], $13::text[], $14::text[],
            $15::text[], $16::text[], $17::text[], $18::float8[], $19::int[], $20::text[],
            $21::int[], $22::int[])
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
            file_modified_at = CASE WHEN $5 THEN $7 ELSE file_modified_at END
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
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Records that ffprobe could not read this file, so an unchanged broken
/// file is skipped by later scans. A changed file clears the mark (see the
/// scan upsert).
pub async fn save_probe_error(pool: &PgPool, item_id: Uuid, error: &str) -> Result<(), DbError> {
    sqlx::query("UPDATE media_items SET probed_at = now(), probe_error = $2 WHERE id = $1")
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
        SELECT container, duration_seconds, bitrate_kbps, file_size, file_modified_at
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
               bitrate_kbps
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
            streams: streams.into_iter().map(MediaStream::from).collect(),
        },
        FileStamp {
            size: facts.file_size,
            modified: facts.file_modified_at,
        },
    )))
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
