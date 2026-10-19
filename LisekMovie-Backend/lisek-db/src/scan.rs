//! Persisting scan results and provider matches into `media_items`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, SubsecRound, Utc};
use serde::Serialize;
use uuid::Uuid;

use lisek_library::{FileFacts, ScanResult};

use crate::{DbError, PgPool};

/// How many of each outcome a scan produced — enough for the admin-facing
/// "scan complete" summary without reporting on every single file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, utoipa::ToSchema)]
pub struct ScanSummary {
    pub movies_found: usize,
    pub episodes_found: usize,
    pub unrecognized: usize,
}

/// What a freshly-upserted `media_items` row still needs looked up from a
/// metadata provider: a movie's title/year, or a series' title.
/// `lisek-db` stays free of any HTTP/provider-API knowledge (see this
/// crate's own doc comment on its boundaries) — [`persist_scan_results`]
/// hands these back to its caller (`lisek-server`, where API keys and the
/// `lisek-metadata` TMDB/TVDB clients live) rather than trying to fetch
/// metadata itself.
///
/// `Hash` lets the enrichment pass look a series up once and reuse the
/// match for every episode of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EnrichTarget {
    Movie { title: String, year: Option<i32> },
    Series { title: String },
}

/// The result of persisting one scan: the admin-facing summary, plus the
/// id, lookup target, and on-disk file path of every item worth a
/// metadata-provider request — which excludes both `Unrecognized` files
/// (nothing to persist for those at all) and any existing row whose
/// metadata stays untouched because `metadata_locked` is set (an admin's
/// manual correction is never overwritten, and is never worth spending a
/// provider request re-confirming either). The file path travels
/// alongside the id/target so the caller can write a provider match's
/// poster/backdrop next to it (see `lisek_metadata::image_cache::
/// save_image`) without a second round trip to look it back up.
pub struct PersistedScan {
    pub summary: ScanSummary,
    /// `(item_id, target, file_path, forced_provider)` — `forced_provider`
    /// is this item's own admin-set override (see the metadata editor's
    /// "Pola aktywne" section), carried straight through from the upsert
    /// below so the caller can skip straight to that one provider instead
    /// of the admin's normal priority list. `None` for the overwhelming
    /// majority of items that have never had one set. Sorted so every
    /// episode of one series is adjacent.
    pub enrich_targets: Vec<(Uuid, EnrichTarget, PathBuf, Option<String>)>,
    /// Items with no stored ffprobe result, or whose file size or
    /// modification time changed since it was taken. Locked items are
    /// included: the lock protects metadata, not technical facts.
    pub probe_targets: Vec<(Uuid, PathBuf)>,
}

/// Rows per `INSERT ... SELECT FROM UNNEST(...)` statement. Large enough
/// that the per-statement series trigger runs rarely, small enough to keep
/// each statement's bind arrays modest.
const PERSIST_BATCH: usize = 500;

/// Persists every [`ScanResult`] from a library scan into `media_items`.
/// See [`persist_scan_results_with_progress`].
pub async fn persist_scan_results(
    pool: &PgPool,
    library_id: Uuid,
    results: &[ScanResult],
) -> Result<PersistedScan, DbError> {
    persist_scan_results_with_progress(pool, library_id, results, |_| {}).await
}

/// Upserts scan results in batches on `(library_id, file_path)`, so
/// re-scanning updates existing rows instead of duplicating them.
/// `Unrecognized` files are counted but never written. `on_batch` receives
/// the number of recognized files written so far.
///
/// Per existing row:
/// - `metadata_locked`: title and metadata stay as they are; only the file
///   facts (size, mtime) are refreshed.
/// - an individual field in `locked_fields` keeps its old value: `title`
///   when `'title'` is locked, `year` when `'year'` is locked.
/// - `original_title` and `provider_data` are carried forward, since the
///   scanner has no fresh value for either.
/// - `probed_at` is cleared when the file's size or mtime changed, which
///   puts the item back on the probe list.
pub async fn persist_scan_results_with_progress(
    pool: &PgPool,
    library_id: Uuid,
    results: &[ScanResult],
    mut on_batch: impl FnMut(usize),
) -> Result<PersistedScan, DbError> {
    let mut summary = ScanSummary::default();
    let mut rows: Vec<ScanRow> = Vec::with_capacity(results.len());
    // Overlapping root paths can report one file twice; a duplicate inside
    // one INSERT ... ON CONFLICT statement is a hard error in Postgres.
    let mut seen: HashSet<&Path> = HashSet::with_capacity(results.len());

    for result in results {
        match result {
            ScanResult::Movie(movie) => {
                if !seen.insert(&movie.file_path) {
                    continue;
                }
                summary.movies_found += 1;
                rows.push(ScanRow {
                    item_type: "movie",
                    title: movie.title.clone(),
                    file_path: movie.file_path.clone(),
                    metadata: serde_json::json!({ "year": movie.year }),
                    file: movie.file,
                    target: EnrichTarget::Movie {
                        title: movie.title.clone(),
                        year: movie.year,
                    },
                });
            }
            ScanResult::Episode(episode) => {
                if !seen.insert(&episode.file_path) {
                    continue;
                }
                summary.episodes_found += 1;
                rows.push(ScanRow {
                    item_type: "episode",
                    title: episode.series_title.clone(),
                    file_path: episode.file_path.clone(),
                    metadata: serde_json::json!({
                        "season": episode.season,
                        "episode": episode.episode,
                        "end_episode": episode.end_episode,
                    }),
                    file: episode.file,
                    target: EnrichTarget::Series {
                        title: episode.series_title.clone(),
                    },
                });
            }
            ScanResult::Unrecognized(_) => summary.unrecognized += 1,
        }
    }

    let mut enrich_targets = Vec::new();
    let mut probe_targets = Vec::new();
    let mut written = 0;
    for chunk in rows.chunks(PERSIST_BATCH) {
        let returned = upsert_batch(pool, library_id, chunk).await?;
        let by_path: HashMap<String, &ScanRow> = chunk
            .iter()
            .map(|row| (row.file_path.to_string_lossy().into_owned(), row))
            .collect();
        for r in returned {
            let Some(row) = by_path.get(&r.file_path) else {
                continue;
            };
            if r.needs_probe {
                probe_targets.push((r.id, row.file_path.clone()));
            }
            if !r.metadata_locked {
                enrich_targets.push((
                    r.id,
                    row.target.clone(),
                    row.file_path.clone(),
                    r.forced_provider,
                ));
            }
        }
        written += chunk.len();
        on_batch(written);
    }

    enrich_targets.sort_by(|a, b| enrich_sort_key(&a.1).cmp(&enrich_sort_key(&b.1)));

    Ok(PersistedScan {
        summary,
        enrich_targets,
        probe_targets,
    })
}

/// Episodes of one series first, grouped; movies after, in title order.
fn enrich_sort_key(target: &EnrichTarget) -> (u8, &str) {
    match target {
        EnrichTarget::Series { title } => (0, title.as_str()),
        EnrichTarget::Movie { title, .. } => (1, title.as_str()),
    }
}

struct ScanRow {
    item_type: &'static str,
    title: String,
    file_path: PathBuf,
    metadata: serde_json::Value,
    file: FileFacts,
    target: EnrichTarget,
}

#[derive(sqlx::FromRow)]
struct UpsertedRow {
    id: Uuid,
    file_path: String,
    forced_provider: Option<String>,
    metadata_locked: bool,
    needs_probe: bool,
}

/// Microsecond precision, matching `timestamptz`. Without the truncation a
/// nanosecond mtime would never compare equal to its stored value and
/// every rescan would re-probe every file.
pub(crate) fn file_timestamp(facts: &FileFacts) -> Option<DateTime<Utc>> {
    facts
        .modified
        .map(|time| DateTime::<Utc>::from(time).trunc_subsecs(6))
}

async fn upsert_batch(
    pool: &PgPool,
    library_id: Uuid,
    rows: &[ScanRow],
) -> Result<Vec<UpsertedRow>, DbError> {
    let item_types: Vec<&str> = rows.iter().map(|r| r.item_type).collect();
    let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    // Lossy for non-UTF-8 paths, exactly like the single-row upsert this
    // replaced; the caller matches RETURNING rows on the same string.
    let paths: Vec<String> = rows
        .iter()
        .map(|r| r.file_path.to_string_lossy().into_owned())
        .collect();
    let metadata: Vec<&serde_json::Value> = rows.iter().map(|r| &r.metadata).collect();
    let sizes: Vec<Option<i64>> = rows
        .iter()
        .map(|r| r.file.size_bytes.and_then(|s| i64::try_from(s).ok()))
        .collect();
    let modified: Vec<Option<DateTime<Utc>>> = rows.iter().map(|r| file_timestamp(&r.file)).collect();

    let returned = sqlx::query_as::<_, UpsertedRow>(
        r#"
        INSERT INTO media_items (library_id, item_type, title, file_path, metadata,
                                 file_size, file_modified_at)
        SELECT $1, t.item_type, t.title, t.file_path, t.metadata, t.file_size, t.file_modified_at
        FROM UNNEST($2::text[], $3::text[], $4::text[], $5::jsonb[], $6::bigint[], $7::timestamptz[])
             AS t(item_type, title, file_path, metadata, file_size, file_modified_at)
        ON CONFLICT (library_id, file_path)
        DO UPDATE SET
            title = CASE
                WHEN media_items.metadata_locked OR 'title' = ANY(media_items.locked_fields)
                    THEN media_items.title
                ELSE EXCLUDED.title
            END,
            metadata = CASE
                WHEN media_items.metadata_locked THEN media_items.metadata
                ELSE EXCLUDED.metadata
                    || CASE WHEN media_items.metadata ? 'original_title'
                            THEN jsonb_build_object('original_title', media_items.metadata->'original_title')
                            ELSE '{}'::jsonb END
                    || CASE WHEN media_items.metadata ? 'provider_data'
                            THEN jsonb_build_object('provider_data', media_items.metadata->'provider_data')
                            ELSE '{}'::jsonb END
                    || CASE WHEN 'year' = ANY(media_items.locked_fields) AND media_items.metadata ? 'year'
                            THEN jsonb_build_object('year', media_items.metadata->'year')
                            ELSE '{}'::jsonb END
            END,
            file_size = EXCLUDED.file_size,
            file_modified_at = EXCLUDED.file_modified_at,
            probed_at = CASE
                WHEN (media_items.file_size, media_items.file_modified_at)
                     IS NOT DISTINCT FROM (EXCLUDED.file_size, EXCLUDED.file_modified_at)
                THEN media_items.probed_at
            END,
            updated_at = CASE WHEN media_items.metadata_locked THEN media_items.updated_at ELSE now() END
        RETURNING id, file_path, forced_provider, metadata_locked, (probed_at IS NULL) AS needs_probe
        "#,
    )
    .bind(library_id)
    .bind(&item_types)
    .bind(&titles)
    .bind(&paths)
    .bind(&metadata)
    .bind(&sizes)
    .bind(&modified)
    .fetch_all(pool)
    .await?;

    Ok(returned)
}

/// Merges a metadata-provider match into an item's `metadata` JSONB. No
/// `metadata_locked` check here — the caller only ever reaches this for an
/// id [`persist_scan_results`] already confirmed was just inserted or
/// unlocked-and-updated, so a lock set concurrently between that call and
/// this one is the only way this could race, an acceptable gap for a
/// same-request follow-up write. Uses `||` (merge), not overwrite, so
/// provider fields sit alongside the scan-derived ones (`year`,
/// `season`/`episode`) already written by the scan upsert rather than
/// replacing them.
///
/// Respects the item's own `locked_fields` — here that's `overview`
/// specifically: when locked, the freshly-fetched `fields.overview` this
/// call was about to write is swapped back out for whatever the item's
/// `provider_data` already had, right inside the query (`locked_fields`
/// and the old `provider_data` are both read live off the row being
/// updated, not passed in from the caller — so this stays correct however
/// the item got locked, whether that was before or after the caller
/// fetched `fields`), so a provider re-match never quietly overwrites an
/// admin's manual description edit. Every other key in `fields`
/// (`tmdb_id`/`tvdb_id`) still updates normally either way — only the
/// single locked key reverts.
pub async fn attach_provider_metadata(
    pool: &PgPool,
    item_id: Uuid,
    provider_key: &str,
    fields: &serde_json::Value,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        UPDATE media_items
        SET metadata = metadata || jsonb_build_object(
                'provider', $2::text,
                'provider_data',
                ($3::jsonb) || CASE
                    WHEN 'overview' = ANY(locked_fields) AND metadata->'provider_data' ? 'overview'
                    THEN jsonb_build_object('overview', metadata->'provider_data'->'overview')
                    ELSE '{}'::jsonb
                END
            ),
            updated_at = now()
        WHERE id = $1
        "#,
    )
    .bind(item_id)
    .bind(provider_key)
    .bind(fields)
    .execute(pool)
    .await?;

    Ok(())
}

/// Records whether an item's provider images ended up in the local
/// config-dir cache only because a media-folder write was attempted and
/// denied (see `lisek_metadata::image_cache::save_image`), rather than
/// because the library simply has media-folder saving turned off. Called
/// once per enriched item, every scan, so a permission problem that gets
/// fixed later and a subsequent successful media-folder write correctly
/// clears the flag again rather than leaving a stale warning behind.
pub async fn set_metadata_fallback(
    pool: &PgPool,
    item_id: Uuid,
    fallback: bool,
) -> Result<(), DbError> {
    // Skips the write (and the row's trigger work) when nothing changes.
    sqlx::query(
        "UPDATE media_items SET metadata_fallback = $2 WHERE id = $1 AND metadata_fallback IS DISTINCT FROM $2",
    )
    .bind(item_id)
    .bind(fallback)
    .execute(pool)
    .await?;

    Ok(())
}

/// Raw row shape for [`list_unlocked_items_for_refresh`]'s query — factored
/// out purely to keep that function's own signature and query-binding line
/// readable (clippy's `type_complexity` lint).
type UnlockedItemRow = (
    Uuid,
    Uuid,
    String,
    String,
    Option<i32>,
    String,
    Option<String>,
);

/// Every item eligible for the background metadata-refresh job: not
/// `metadata_locked` (the same gate the scan upsert already applies — a
/// locked item is exempt from automatic changes either way), enough to
/// rebuild its [`EnrichTarget`], and its `forced_provider` so the refresh
/// job can respect it exactly the way a scan's own enrichment pass does.
/// Individual `locked_fields` need no equivalent here —
/// [`attach_provider_metadata`] reads that column live off the row it's
/// updating, not from anything this function hands back. `library_id`
/// rides along so the caller can look up each item's library (for
/// `save_metadata_in_media_folder` and any per-library provider order)
/// without a second per-item query.
///
/// Ordered so every episode of a series is adjacent: the refresh pass then
/// looks each series up once instead of once per episode.
pub async fn list_unlocked_items_for_refresh(
    pool: &PgPool,
) -> Result<Vec<(Uuid, Uuid, EnrichTarget, PathBuf, Option<String>)>, DbError> {
    let rows: Vec<UnlockedItemRow> = sqlx::query_as(
        r#"
        SELECT id, library_id, item_type, title, year, file_path, forced_provider
        FROM media_items
        WHERE NOT metadata_locked
        ORDER BY item_type, library_id, title, id
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(id, library_id, item_type, title, year, file_path, forced_provider)| {
                let target = if item_type == "movie" {
                    EnrichTarget::Movie { title, year }
                } else {
                    EnrichTarget::Series { title }
                };
                (
                    id,
                    library_id,
                    target,
                    PathBuf::from(file_path),
                    forced_provider,
                )
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn file_timestamps_are_truncated_to_postgres_precision() {
        let facts = FileFacts {
            size_bytes: Some(1),
            modified: Some(UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789)),
        };
        let stamp = file_timestamp(&facts).unwrap();
        assert_eq!(stamp.timestamp_subsec_nanos(), 123_456_000);
    }

    #[test]
    fn enrich_targets_group_series_before_movies() {
        let mut targets = [
            EnrichTarget::Movie { title: "A".into(), year: None },
            EnrichTarget::Series { title: "Z".into() },
            EnrichTarget::Series { title: "B".into() },
        ];
        targets.sort_by(|a, b| enrich_sort_key(a).cmp(&enrich_sort_key(b)));
        assert_eq!(targets[0], EnrichTarget::Series { title: "B".into() });
        assert_eq!(targets[2], EnrichTarget::Movie { title: "A".into(), year: None });
    }
}
