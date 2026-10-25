//! Trickplay: seek-bar preview thumbnails.
//!
//! A background worker (started once from `main`, woken after every scan)
//! picks probed movies and episodes without current thumbnails, newest
//! first, and runs one FFmpeg job at a time (`lisek_playback::trickplay`).
//! It waits while anyone is transcoding, so previews never cost a viewer
//! smooth playback. Sheets are written next to the image cache in the
//! config directory (`trickplay/<item id>/sheet-NNN.jpg`): a new set is
//! built in `<item id>.tmp` and swapped in only when complete.
//!
//! `LISEK_TRICKPLAY=0` turns generation off (thumbnails already made are
//! still served).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use lisek_core::playback_settings::PlaybackSettings;
use lisek_db::PgPool;
use lisek_db::trickplay::{
    self as db, LibraryTrickplayStats, TrickplayCandidate, TrickplayFailure, TrickplayReady,
    TrickplayStats,
};
use lisek_playback::session;
use lisek_playback::transcode::{self, FfmpegTools};
use lisek_playback::trickplay::{self, Plan, TrickplayError};
use serde::Serialize;
use uuid::Uuid;

use crate::auth::{AdminUser, AuthUser};
use crate::playback::{PlaybackError, pool};
use crate::state::{SharedState, config_dir};

/// Items fetched per round; the worker re-checks for transcodes between items.
const BATCH: i64 = 8;
/// Sleep when there is nothing to do (a finished scan wakes it earlier).
const IDLE_RECHECK: Duration = Duration::from_secs(15 * 60);
/// Re-check interval while someone is transcoding.
const BUSY_RECHECK: Duration = Duration::from_secs(30);

/// Generation times kept for the ETA estimate.
const TIMING_WINDOW: usize = 20;

/// The worker's live state for the admin view. Lives in memory: after a
/// restart it starts empty.
#[derive(Debug, Default)]
pub struct Progress {
    current: Option<CurrentJob>,
    /// Seconds per item of the latest generations, newest last.
    recent: VecDeque<f64>,
    /// FFmpeg could not be started; generation waits for a configuration
    /// change.
    ffmpeg_missing: bool,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct CurrentJob {
    pub item_id: Uuid,
    pub title: String,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl Progress {
    fn average_seconds(&self) -> Option<f64> {
        (!self.recent.is_empty()).then(|| self.recent.iter().sum::<f64>() / self.recent.len() as f64)
    }
}

fn set_progress(state: &SharedState, update: impl FnOnce(&mut Progress)) {
    if let Ok(mut progress) = state.trickplay_progress.lock() {
        update(&mut progress);
    }
}

fn root() -> PathBuf {
    config_dir().join("trickplay")
}

/// `LISEK_TRICKPLAY=0|false|off|no` disables generation.
pub fn enabled() -> bool {
    !std::env::var("LISEK_TRICKPLAY")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(false)
}

async fn wait(state: &SharedState, timeout: Duration) {
    tokio::select! {
        _ = state.trickplay_wakeup.notified() => {}
        _ = tokio::time::sleep(timeout) => {}
    }
}

/// Runs for the life of the process.
pub async fn run_worker(state: SharedState) {
    if !enabled() {
        tracing::info!("trickplay generation is off (LISEK_TRICKPLAY)");
        return;
    }
    let mut swept = false;
    loop {
        let Some(pool) = state.pool() else {
            wait(&state, IDLE_RECHECK).await;
            continue;
        };
        if !swept {
            sweep(&pool).await;
            swept = true;
        }
        if session::encoding_sessions() > 0 {
            tokio::time::sleep(BUSY_RECHECK).await;
            continue;
        }
        let candidates = match db::trickplay_candidates(&pool, BATCH).await {
            Ok(candidates) => candidates,
            Err(err) => {
                tracing::warn!(%err, "trickplay queue unavailable");
                tokio::time::sleep(BUSY_RECHECK).await;
                continue;
            }
        };
        if candidates.is_empty() {
            wait(&state, IDLE_RECHECK).await;
            continue;
        }
        let settings = match lisek_db::playback::settings(&pool).await {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(%err, "cannot read playback settings for trickplay");
                tokio::time::sleep(BUSY_RECHECK).await;
                continue;
            }
        };
        let tools = transcode::tools(&settings).await;
        if tools.version.is_empty() {
            tracing::warn!(
                "FFmpeg is not available; trickplay thumbnails wait until it is configured"
            );
            set_progress(&state, |p| p.ffmpeg_missing = true);
            wait(&state, IDLE_RECHECK).await;
            continue;
        }
        set_progress(&state, |p| p.ffmpeg_missing = false);
        for candidate in candidates {
            if session::encoding_sessions() > 0 {
                break;
            }
            let job = CurrentJob {
                item_id: candidate.id,
                title: candidate.title.clone(),
                season: candidate.season_number,
                episode: candidate.episode_number,
                started_at: chrono::Utc::now(),
            };
            set_progress(&state, |p| p.current = Some(job));
            let started = Instant::now();
            let outcome = process(&pool, &settings, &tools, candidate).await;
            let elapsed = started.elapsed().as_secs_f64();
            set_progress(&state, |p| {
                p.current = None;
                if outcome == Outcome::Done {
                    p.recent.push_back(elapsed);
                    while p.recent.len() > TIMING_WINDOW {
                        p.recent.pop_front();
                    }
                }
                p.ffmpeg_missing = outcome == Outcome::FfmpegMissing;
            });
            if outcome == Outcome::FfmpegMissing {
                wait(&state, IDLE_RECHECK).await;
                break;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Sheets made.
    Done,
    /// This item failed (recorded); carry on with the next.
    Failed,
    /// FFmpeg itself cannot run: nothing else can succeed until the
    /// configuration changes.
    FfmpegMissing,
}

/// Generates one item.
async fn process(
    pool: &PgPool,
    settings: &PlaybackSettings,
    tools: &FfmpegTools,
    item: TrickplayCandidate,
) -> Outcome {
    let started = Instant::now();
    let size = |v: Option<i32>| v.unwrap_or(0).max(0) as u32;
    let Some(plan) = Plan::new(item.duration_seconds, size(item.width), size(item.height)) else {
        failed(pool, &item, "no picture size or duration").await;
        return Outcome::Failed;
    };
    let input = PathBuf::from(&item.file_path);
    if !tokio::fs::try_exists(&input).await.unwrap_or(false) {
        failed(pool, &item, "file not found").await;
        return Outcome::Failed;
    }
    let id = item.id.to_string();
    let target = trickplay::item_dir(&root(), &id);
    let building = root().join(format!("{id}.tmp"));
    let _ = tokio::fs::remove_dir_all(&building).await;
    match trickplay::generate(
        settings,
        tools,
        &input,
        &building,
        &plan,
        item.color_transfer.as_deref(),
    )
    .await
    {
        Ok(sheets) => {
            let _ = tokio::fs::remove_dir_all(&target).await;
            if let Err(err) = tokio::fs::rename(&building, &target).await {
                let _ = tokio::fs::remove_dir_all(&building).await;
                failed(pool, &item, &format!("cannot store thumbnails: {err}")).await;
                return Outcome::Failed;
            }
            let count = plan.count.min(sheets * plan.per_sheet());
            let ready = TrickplayReady {
                item_id: item.id,
                interval_seconds: plan.interval as i32,
                tile_width: plan.width as i32,
                tile_height: plan.height as i32,
                grid_columns: plan.columns as i32,
                grid_rows: plan.rows as i32,
                thumbnail_count: count as i32,
                sheet_count: sheets as i32,
                source_size: item.file_size,
                source_modified_at: item.file_modified_at.as_ref(),
            };
            if let Err(err) = db::save_trickplay_ready(pool, &ready).await {
                tracing::warn!(item_id = %item.id, %err, "cannot save trickplay");
            }
            tracing::info!(
                item_id = %item.id,
                sheets,
                thumbnails = count,
                interval = plan.interval,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "trickplay thumbnails ready"
            );
            Outcome::Done
        }
        Err(TrickplayError::Spawn(err)) => {
            let _ = tokio::fs::remove_dir_all(&building).await;
            tracing::warn!(%err, "cannot start FFmpeg for trickplay");
            Outcome::FfmpegMissing
        }
        Err(err) => {
            let _ = tokio::fs::remove_dir_all(&building).await;
            tracing::warn!(item_id = %item.id, file = %item.file_path, %err, "trickplay generation failed");
            failed(pool, &item, &err.to_string()).await;
            Outcome::Failed
        }
    }
}

async fn failed(pool: &PgPool, item: &TrickplayCandidate, error: &str) {
    if let Err(err) = db::save_trickplay_failed(
        pool,
        item.id,
        error,
        item.file_size,
        item.file_modified_at.as_ref(),
    )
    .await
    {
        tracing::warn!(item_id = %item.id, %err, "cannot save trickplay failure");
    }
}

/// Removes half-built sets and sets of items that no longer have thumbnails
/// (deleted items, regenerations).
async fn sweep(pool: &PgPool) {
    let Ok(ready) = db::ready_trickplay_ids(pool).await else {
        return;
    };
    let ready: std::collections::HashSet<String> =
        ready.into_iter().map(|id| id.to_string()).collect();
    let Ok(mut entries) = tokio::fs::read_dir(root()).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !ready.contains(&name) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

/// What a player needs to show previews: tile geometry and sheet URLs.
/// Thumbnail `n` (the moment `n * interval_seconds`) is on sheet
/// `n / (columns * rows)`, column `n % columns`, row
/// `(n % (columns * rows)) / columns`.
#[derive(Serialize, utoipa::ToSchema)]
pub struct TrickplayManifest {
    pub interval_seconds: i32,
    /// Tile size in pixels.
    pub width: i32,
    pub height: i32,
    pub columns: i32,
    pub rows: i32,
    /// Thumbnails in total (the last sheet may be partly empty).
    pub count: i32,
    /// Sheet image URLs in order; like other images they take `?token=`.
    pub sheets: Vec<String>,
}

fn not_ready() -> PlaybackError {
    PlaybackError(StatusCode::NOT_FOUND, "trickplay_not_ready")
}

/// Preview thumbnails of an item; 404 `trickplay_not_ready` until the
/// background job has made them.
#[utoipa::path(get,path="/api/media/{item_id}/trickplay",tag="media",params(("item_id"=Uuid,Path)),responses((status=200,body=TrickplayManifest),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn manifest(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<TrickplayManifest>, PlaybackError> {
    let pool = pool(&state).await?;
    if !auth.can_see(&pool, item_id).await? {
        return Err(not_ready());
    }
    let row = db::get_trickplay(&pool, item_id)
        .await?
        .ok_or_else(not_ready)?;
    // Part of every URL: a regenerated set never comes from a stale cache.
    let version = row.generated_at.timestamp();
    Ok(Json(TrickplayManifest {
        interval_seconds: row.interval_seconds,
        width: row.tile_width,
        height: row.tile_height,
        columns: row.grid_columns,
        rows: row.grid_rows,
        count: row.thumbnail_count,
        sheets: (0..row.sheet_count)
            .map(|n| format!("/api/media/{item_id}/trickplay/{n}?v={version}"))
            .collect(),
    }))
}

/// One sheet (JPEG). Cached by the browser for good: its URL changes when
/// the set is regenerated.
#[utoipa::path(get,path="/api/media/{item_id}/trickplay/{sheet}",tag="media",params(("item_id"=Uuid,Path),("sheet"=u32,Path)),responses((status=200,description="JPEG sprite sheet"),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn sheet(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path((item_id, sheet)): Path<(Uuid, u32)>,
) -> Result<Response, PlaybackError> {
    let pool = pool(&state).await?;
    if !auth.can_see(&pool, item_id).await? {
        return Err(not_ready());
    }
    let row = db::get_trickplay(&pool, item_id)
        .await?
        .ok_or_else(not_ready)?;
    if i64::from(sheet) >= i64::from(row.sheet_count) {
        return Err(not_ready());
    }
    let path =
        trickplay::item_dir(&root(), &item_id.to_string()).join(trickplay::sheet_name(sheet));
    let bytes = tokio::fs::read(&path).await.map_err(|_| not_ready())?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable",
            ),
        ],
        bytes,
    )
        .into_response())
}

/// Throws an item's thumbnails away and queues it again (after replacing a
/// file in place, or to retry a failure).
#[utoipa::path(post,path="/api/admin/media/{item_id}/trickplay",tag="admin",params(("item_id"=Uuid,Path)),responses((status=202),(status=401),(status=403),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn regenerate(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<StatusCode, PlaybackError> {
    if !db::reset_trickplay(&pool(&state).await?, item_id).await? {
        return Err(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"));
    }
    let _ = tokio::fs::remove_dir_all(trickplay::item_dir(&root(), &item_id.to_string())).await;
    state.trickplay_wakeup.notify_one();
    Ok(StatusCode::ACCEPTED)
}

/// How many failures [`status`] lists.
const FAILURE_LIST: i64 = 50;

#[derive(Serialize, utoipa::ToSchema)]
pub struct TrickplayStatus {
    /// Generation runs (`LISEK_TRICKPLAY` not set to 0).
    pub enabled: bool,
    /// Generation waits because someone is transcoding.
    pub paused_for_playback: bool,
    /// FFmpeg could not be started (check Admin → Playback).
    pub ffmpeg_missing: bool,
    #[serde(flatten)]
    pub counts: TrickplayStats,
    /// The item being generated right now.
    pub current: Option<CurrentJob>,
    /// Average seconds per item over the latest generations (since start).
    pub average_seconds: Option<f64>,
    /// `pending * average_seconds`, when an average exists.
    pub eta_seconds: Option<f64>,
    pub libraries: Vec<LibraryTrickplayStats>,
    /// Latest failures, newest first.
    pub failures: Vec<TrickplayFailure>,
}

/// How far thumbnail generation has got: totals, per library, the item in
/// progress and recent failures. The admin page polls it.
#[utoipa::path(get,path="/api/admin/trickplay",tag="admin",responses((status=200,body=TrickplayStatus),(status=401),(status=403),(status=503)),security(("lisek_auth"=[])))]
pub async fn status(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<TrickplayStatus>, PlaybackError> {
    let pool = pool(&state).await?;
    let (counts, libraries, failures) = tokio::try_join!(
        db::trickplay_stats(&pool),
        db::trickplay_library_stats(&pool),
        db::trickplay_failures(&pool, FAILURE_LIST),
    )?;
    let (current, average_seconds, ffmpeg_missing) = state
        .trickplay_progress
        .lock()
        .map(|p| (p.current.clone(), p.average_seconds(), p.ffmpeg_missing))
        .unwrap_or_default();
    let enabled = enabled();
    Ok(Json(TrickplayStatus {
        enabled,
        paused_for_playback: enabled && session::encoding_sessions() > 0,
        ffmpeg_missing,
        eta_seconds: average_seconds.map(|avg| avg * counts.pending as f64),
        counts,
        current,
        average_seconds,
        libraries,
        failures,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_covers_the_latest_generations() {
        let mut progress = Progress::default();
        assert_eq!(progress.average_seconds(), None);
        progress.recent.extend([10.0, 20.0, 30.0]);
        assert_eq!(progress.average_seconds(), Some(20.0));
    }
}
