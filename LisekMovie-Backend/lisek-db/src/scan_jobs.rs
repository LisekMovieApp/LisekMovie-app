//! Persistent scan queue (`scan_jobs`). The table is the queue: a job
//! survives a server restart, and the partial unique index allows only one
//! pending job per library.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{DbError, PgPool};

/// One scan, as stored and as reported to clients (HTTP and WebSocket).
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ScanJob {
    pub id: Uuid,
    pub library_id: Uuid,
    /// `queued` | `running` | `completed` | `failed`
    pub status: String,
    /// `queued` | `walking` | `saving` | `enriching` | `probing` | `done`
    pub phase: String,
    pub phase_done: i32,
    pub phase_total: i32,
    pub files_found: i32,
    pub movies_found: i32,
    pub episodes_found: i32,
    pub unrecognized: i32,
    pub probed: i32,
    pub probe_failed: i32,
    pub metadata_matched: i32,
    pub images_saved: i32,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl ScanJob {
    pub fn is_finished(&self) -> bool {
        matches!(self.status.as_str(), "completed" | "failed")
    }
}

const COLUMNS: &str = "id, library_id, status, phase, phase_done, phase_total, files_found, \
    movies_found, episodes_found, unrecognized, probed, probe_failed, metadata_matched, \
    images_saved, error, created_at, started_at, finished_at";

/// Queues a scan of `library_id`, or returns the scan already pending for
/// it. The boolean is `true` when a new job was created.
pub async fn enqueue(
    pool: &PgPool,
    library_id: Uuid,
    requested_by: Option<Uuid>,
) -> Result<(ScanJob, bool), DbError> {
    let created: Option<ScanJob> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO scan_jobs (library_id, requested_by) VALUES ($1, $2) \
         ON CONFLICT (library_id) WHERE status IN ('queued', 'running') DO NOTHING \
         RETURNING {COLUMNS}"
    )))
    .bind(library_id)
    .bind(requested_by)
    .fetch_optional(pool)
    .await?;
    if let Some(job) = created {
        return Ok((job, true));
    }

    let active: Option<ScanJob> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM scan_jobs \
         WHERE library_id = $1 AND status IN ('queued', 'running')"
    )))
    .bind(library_id)
    .fetch_optional(pool)
    .await?;
    // The pending job can finish between the two statements; queue again.
    match active {
        Some(job) => Ok((job, false)),
        None => Box::pin(enqueue(pool, library_id, requested_by)).await,
    }
}

pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<ScanJob>, DbError> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM scan_jobs WHERE id = $1"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// The newest jobs first, across all libraries.
pub async fn list_recent(pool: &PgPool, limit: i64) -> Result<Vec<ScanJob>, DbError> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM scan_jobs ORDER BY created_at DESC LIMIT $1"
    )))
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// Marks the oldest queued job as running and returns it. `SKIP LOCKED`
/// keeps this safe should more than one worker ever run.
pub async fn claim_next(pool: &PgPool) -> Result<Option<ScanJob>, DbError> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE scan_jobs SET status = 'running', phase = 'walking', started_at = now() \
         WHERE id = (SELECT id FROM scan_jobs WHERE status = 'queued' \
                     ORDER BY created_at LIMIT 1 FOR UPDATE SKIP LOCKED) \
         RETURNING {COLUMNS}"
    )))
    .fetch_optional(pool)
    .await?)
}

/// Puts jobs a previous process left `running` back in the queue. Called
/// once when the worker starts.
pub async fn requeue_interrupted(pool: &PgPool) -> Result<u64, DbError> {
    Ok(sqlx::query(
        "UPDATE scan_jobs SET status = 'queued', phase = 'queued', phase_done = 0, \
         phase_total = 0, started_at = NULL WHERE status = 'running'",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

/// Writes every mutable field of `job`. The worker keeps the job in memory
/// and saves it at phase changes and on a timer, not per file.
pub async fn save(pool: &PgPool, job: &ScanJob) -> Result<(), DbError> {
    sqlx::query(
        r#"
        UPDATE scan_jobs SET
            status = $2, phase = $3, phase_done = $4, phase_total = $5, files_found = $6,
            movies_found = $7, episodes_found = $8, unrecognized = $9, probed = $10,
            probe_failed = $11, metadata_matched = $12, images_saved = $13, error = $14,
            finished_at = $15
        WHERE id = $1
        "#,
    )
    .bind(job.id)
    .bind(&job.status)
    .bind(&job.phase)
    .bind(job.phase_done)
    .bind(job.phase_total)
    .bind(job.files_found)
    .bind(job.movies_found)
    .bind(job.episodes_found)
    .bind(job.unrecognized)
    .bind(job.probed)
    .bind(job.probe_failed)
    .bind(job.metadata_matched)
    .bind(job.images_saved)
    .bind(&job.error)
    .bind(job.finished_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// UI language of the admin who queued the job; providers use it to pick
/// image and text language. `None` when the account no longer exists.
pub async fn requester_language(pool: &PgPool, job_id: Uuid) -> Result<Option<String>, DbError> {
    Ok(sqlx::query_scalar(
        "SELECT u.ui_language FROM scan_jobs j JOIN users u ON u.id = j.requested_by WHERE j.id = $1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?)
}

/// Deletes finished jobs older than `days`. Called after each scan so the
/// table stays small without a separate maintenance task.
pub async fn prune_finished(pool: &PgPool, days: i32) -> Result<u64, DbError> {
    Ok(sqlx::query(
        "DELETE FROM scan_jobs WHERE status IN ('completed', 'failed') \
         AND finished_at < now() - make_interval(days => $1)",
    )
    .bind(days)
    .execute(pool)
    .await?
    .rows_affected())
}
