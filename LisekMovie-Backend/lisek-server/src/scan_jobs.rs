//! Background library scans.
//!
//! `POST /api/admin/libraries/{id}/scan` only queues a job (`scan_jobs`
//! table) and returns 202. One worker task drains the queue, one job at a
//! time, through these phases:
//!
//! 1. `walking`   — filesystem walk on a blocking thread (`spawn_blocking`),
//!    so a slow network share never stalls the async runtime;
//! 2. `saving`    — batched upserts (`lisek_db::persist_scan_results_*`);
//! 3. `enriching` — metadata providers, one lookup per series;
//! 4. `probing`   — ffprobe for new or changed files only, results stored
//!    in `media_streams`, a few files in parallel;
//! 5. `done`.
//!
//! Progress is written to the job row and broadcast as a `scan_progress`
//! event to admin WebSockets at most once per [`PROGRESS_INTERVAL`] and on
//! every phase change. A job left `running` by a crash is re-queued when the
//! worker starts.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{Datelike, Utc};
use lisek_core::media_probe::MediaProbe;
use lisek_db::PgPool;
use lisek_db::scan_jobs::{self as db, ScanJob};
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::admin::{
    AdminError, apply_provider_order, item_type_for_library, map_db_error, require_admin,
};
use crate::auth::AuthUser;
use crate::metadata_enrichment::{self, EnrichCache};
use crate::state::{ServerEvent, SharedState};

const PROGRESS_INTERVAL: Duration = Duration::from_secs(1);
/// Parallel ffprobe processes. Probing is I/O-bound (it reads headers,
/// often over the network); a handful overlaps latency without thrashing
/// a spinning disk.
const PROBE_CONCURRENCY: usize = 4;
/// Fallback poll when no wake-up arrives (e.g. a job queued while the
/// server was still in setup mode).
const IDLE_RECHECK: Duration = Duration::from_secs(60);
/// Finished jobs are kept this long for the admin panel's history.
const KEEP_FINISHED_DAYS: i32 = 30;

/// Queues a scan of one library. Returns the job immediately (202); an
/// already pending scan of the same library is returned instead of a
/// second one. Follow progress via `GET /api/admin/scan-jobs/{id}` or the
/// `scan_progress` events on `/api/notifications/socket`.
#[utoipa::path(post,path="/api/admin/libraries/{library_id}/scan",tag="admin",params(("library_id"=Uuid,Path)),responses((status=202,body=ScanJob),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn scan_library(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<(StatusCode, Json<ScanJob>), AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;
    lisek_db::get_library(&pool, library_id)
        .await
        .map_err(map_db_error)?
        .ok_or(AdminError::LibraryNotFound)?;

    let (job, created) = db::enqueue(&pool, library_id, Some(auth.user.id))
        .await
        .map_err(map_db_error)?;
    if created {
        tracing::info!(%library_id, job_id = %job.id, admin = %auth.user.username, "library scan queued");
        state.scan_wakeup.notify_one();
    }
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// The 50 most recent scan jobs, newest first.
#[utoipa::path(get,path="/api/admin/scan-jobs",tag="admin",responses((status=200,body=Vec<ScanJob>),(status=401),(status=403),(status=503)),security(("lisek_auth"=[])))]
pub async fn list(
    auth: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<ScanJob>>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;
    Ok(Json(db::list_recent(&pool, 50).await.map_err(map_db_error)?))
}

#[utoipa::path(get,path="/api/admin/scan-jobs/{job_id}",tag="admin",params(("job_id"=Uuid,Path)),responses((status=200,body=ScanJob),(status=401),(status=403),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn get(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<ScanJob>, AdminError> {
    require_admin(&auth)?;
    let pool = state.pool().await.ok_or(AdminError::ServerNotReady)?;
    db::get(&pool, job_id)
        .await
        .map_err(map_db_error)?
        .map(Json)
        .ok_or(AdminError::ItemNotFound)
}

/// Runs for the life of the process. Started once from `main`.
pub async fn run_worker(state: SharedState) {
    let mut recovered = false;
    loop {
        let Some(pool) = state.pool().await else {
            wait_for_work(&state).await;
            continue;
        };
        if !recovered {
            match db::requeue_interrupted(&pool).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(jobs = n, "re-queued scans interrupted by a restart"),
                Err(err) => tracing::warn!(%err, "could not re-queue interrupted scans"),
            }
            recovered = true;
        }
        match db::claim_next(&pool).await {
            Ok(Some(job)) => run_job(&state, &pool, job).await,
            Ok(None) => wait_for_work(&state).await,
            Err(err) => {
                tracing::warn!(%err, "scan queue unavailable");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

async fn wait_for_work(state: &SharedState) {
    tokio::select! {
        _ = state.scan_wakeup.notified() => {}
        _ = tokio::time::sleep(IDLE_RECHECK) => {}
    }
}

/// The in-memory job plus throttled persistence and broadcasting.
struct Progress<'a> {
    state: &'a SharedState,
    pool: &'a PgPool,
    job: ScanJob,
    last_flush: Instant,
}

impl Progress<'_> {
    async fn phase(&mut self, phase: &str, total: usize) {
        self.job.phase = phase.to_owned();
        self.job.phase_done = 0;
        self.job.phase_total = clamp(total);
        self.flush().await;
    }

    async fn tick(&mut self, done: usize) {
        self.job.phase_done = clamp(done);
        if self.last_flush.elapsed() >= PROGRESS_INTERVAL {
            self.flush().await;
        }
    }

    async fn flush(&mut self) {
        if let Err(err) = db::save(self.pool, &self.job).await {
            tracing::warn!(job_id = %self.job.id, %err, "failed to save scan progress");
        }
        // No receivers is normal (no admin connected).
        let _ = self.state.events.send(ServerEvent::ScanProgress {
            job: self.job.clone(),
        });
        self.last_flush = Instant::now();
    }
}

fn clamp(value: usize) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

async fn run_job(state: &SharedState, pool: &PgPool, job: ScanJob) {
    let started = Instant::now();
    let mut progress = Progress {
        state,
        pool,
        job,
        last_flush: Instant::now(),
    };
    progress.flush().await;

    let outcome = execute(&mut progress).await;
    let job = &mut progress.job;
    match &outcome {
        Ok(()) => {
            job.status = "completed".into();
            job.phase = "done".into();
            job.phase_done = job.phase_total;
        }
        Err(error) => {
            job.status = "failed".into();
            job.error = Some(error.clone());
        }
    }
    job.finished_at = Some(Utc::now());
    progress.flush().await;

    if let Err(err) = db::prune_finished(pool, KEEP_FINISHED_DAYS).await {
        tracing::warn!(%err, "failed to prune old scan jobs");
    }

    let job = &progress.job;
    tracing::info!(
        job_id = %job.id,
        library_id = %job.library_id,
        status = %job.status,
        error = ?job.error,
        files_found = job.files_found,
        movies_found = job.movies_found,
        episodes_found = job.episodes_found,
        unrecognized = job.unrecognized,
        metadata_matched = job.metadata_matched,
        images_saved = job.images_saved,
        probed = job.probed,
        probe_failed = job.probe_failed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "library scan finished"
    );
}

/// Polls `counter` into the job's progress until `task` finishes.
async fn drive<T>(
    progress: &mut Progress<'_>,
    counter: &AtomicUsize,
    mut task: tokio::task::JoinHandle<T>,
) -> Result<T, String> {
    loop {
        tokio::select! {
            result = &mut task => return result.map_err(|_| "scan_task_failed".to_string()),
            _ = tokio::time::sleep(PROGRESS_INTERVAL) => {
                progress.tick(counter.load(Ordering::Relaxed)).await;
            }
        }
    }
}

async fn execute(progress: &mut Progress<'_>) -> Result<(), String> {
    let pool = progress.pool.clone();
    let library_id = progress.job.library_id;
    let library = lisek_db::get_library(&pool, library_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("library_not_found")?;
    let library_type = lisek_library::LibraryType::from_db_str(&library.library_type)
        .ok_or("invalid_library_type")?;

    // 1. Walk the disk on a blocking thread.
    progress.phase("walking", 0).await;
    let found = Arc::new(AtomicUsize::new(0));
    let walk = {
        let found = found.clone();
        let roots = library.root_paths.clone();
        let now_year = Utc::now().year();
        tokio::task::spawn_blocking(move || {
            lisek_library::scan_library_with_progress(&roots, library_type, now_year, |n| {
                found.store(n, Ordering::Relaxed)
            })
        })
    };
    let results = Arc::new(drive(progress, &found, walk).await?);
    progress.job.files_found = clamp(results.len());

    // 2. Persist in batches.
    let recognized = results
        .iter()
        .filter(|r| !matches!(r, lisek_library::ScanResult::Unrecognized(_)))
        .count();
    progress.phase("saving", recognized).await;
    let saved = Arc::new(AtomicUsize::new(0));
    let persist = {
        let saved = saved.clone();
        let results = results.clone();
        let pool = pool.clone();
        tokio::spawn(async move {
            lisek_db::persist_scan_results_with_progress(&pool, library_id, &results, |n| {
                saved.store(n, Ordering::Relaxed)
            })
            .await
        })
    };
    let persisted = drive(progress, &saved, persist)
        .await?
        .map_err(|e| e.to_string())?;
    drop(results);
    progress.job.movies_found = clamp(persisted.summary.movies_found);
    progress.job.episodes_found = clamp(persisted.summary.episodes_found);
    progress.job.unrecognized = clamp(persisted.summary.unrecognized);

    // 3. Metadata providers, before probing: posters are what users notice.
    enrich(progress, &library, &persisted.enrich_targets).await?;

    // 4. ffprobe for new or changed files.
    probe(progress, persisted.probe_targets).await;
    Ok(())
}

async fn enrich(
    progress: &mut Progress<'_>,
    library: &lisek_core::Library,
    targets: &[(Uuid, lisek_db::EnrichTarget, PathBuf, Option<String>)],
) -> Result<(), String> {
    let pool = progress.pool.clone();
    let global = lisek_db::list_enabled_provider_keys(&pool)
        .await
        .map_err(|e| e.to_string())?;
    // A library can override the admin's global priority order for its own
    // item type; a provider disabled since that order was saved drops out.
    let item_type = item_type_for_library(&library.library_type);
    let library_order = lisek_db::get_library_provider_order(&pool, library.id, item_type)
        .await
        .map_err(|e| e.to_string())?;
    let providers = apply_provider_order(global, library_order.as_deref());
    if providers.is_empty() {
        tracing::warn!(
            library_id = %library.id,
            "no enabled/configured metadata provider — skipping enrichment for this scan; \
             set an API key for at least one provider under Admin Panel \u{2192} Plugins"
        );
        return Ok(());
    }
    if targets.is_empty() {
        return Ok(());
    }

    // No server-wide metadata language exists yet; the requesting admin's
    // UI language stands in for it, as it did when scans were synchronous.
    let language = db::requester_language(&pool, progress.job.id)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "en".to_owned());
    let http = reqwest::Client::new();
    let config_dir = crate::state::config_dir();
    let mut cache = EnrichCache::default();

    progress.phase("enriching", targets.len()).await;
    for (done, (item_id, target, file_path, forced_provider)) in targets.iter().enumerate() {
        let outcome = metadata_enrichment::enrich_item_cached(
            &mut cache,
            &pool,
            &http,
            &config_dir,
            &providers,
            *item_id,
            target,
            file_path,
            library.save_metadata_in_media_folder,
            &language,
            forced_provider.as_deref(),
        )
        .await;
        progress.job.metadata_matched += i32::from(outcome.matched);
        progress.job.images_saved += i32::from(outcome.image_saved);
        progress.tick(done + 1).await;
    }
    Ok(())
}

type ProbeResult = (Uuid, Result<MediaProbe, String>);

async fn probe(progress: &mut Progress<'_>, targets: Vec<(Uuid, PathBuf)>) {
    if targets.is_empty() {
        return;
    }
    let pool = progress.pool.clone();
    let settings = match lisek_db::playback::settings(&pool).await {
        Ok(settings) => Arc::new(settings),
        Err(err) => {
            tracing::warn!(%err, "cannot read playback settings; skipping ffprobe for this scan");
            return;
        }
    };

    progress.phase("probing", targets.len()).await;
    let mut pending = targets.into_iter();
    let mut running: JoinSet<ProbeResult> = JoinSet::new();
    let mut done = 0usize;
    let spawn = |running: &mut JoinSet<ProbeResult>, (id, path): (Uuid, PathBuf)| {
        let settings = settings.clone();
        running.spawn(async move {
            (id, lisek_playback::transcode::probe_media(&path, &settings).await)
        });
    };
    for target in pending.by_ref().take(PROBE_CONCURRENCY) {
        spawn(&mut running, target);
    }

    while let Some(joined) = running.join_next().await {
        let Ok((item_id, result)) = joined else {
            continue;
        };
        match result {
            Ok(probe) => match lisek_db::streams::save_probe(&pool, item_id, &probe, None).await {
                Ok(()) => progress.job.probed += 1,
                Err(err) => tracing::warn!(%item_id, %err, "failed to store probe result"),
            },
            Err(reason) if reason == "ffprobe_unavailable" => {
                // A configuration problem, not a property of the file: do not
                // mark anything, playback will probe lazily once fixed.
                tracing::warn!(
                    "ffprobe is not available (Admin Panel \u{2192} Playback); \
                     skipping stream analysis for this scan"
                );
                running.abort_all();
                return;
            }
            Err(reason) => {
                progress.job.probe_failed += 1;
                if let Err(err) = lisek_db::streams::save_probe_error(&pool, item_id, &reason).await {
                    tracing::warn!(%item_id, %err, "failed to record probe error");
                }
            }
        }
        done += 1;
        progress.tick(done).await;
        if let Some(next) = pending.next() {
            spawn(&mut running, next);
        }
    }
}
