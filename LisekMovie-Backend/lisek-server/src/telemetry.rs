//! Playback telemetry endpoints: progress reports from players (resume
//! position, viewing time, quality counters) and the admin view of recent
//! sessions. The rules live in `lisek-telemetry`, persistence in
//! `lisek_db::playback`.

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use lisek_db::playback as db;
use lisek_playback::session;
use lisek_telemetry::{PlaybackProgress, ProgressAck, ProgressEvent};
use serde::Deserialize;
use uuid::Uuid;

use crate::auth::{AdminUser, AuthUser};
use crate::playback::{PlaybackError, pool};
use crate::state::SharedState;

/// Applies one report: stores the position and counters, keeps the HLS
/// session alive or ends it.
pub(crate) async fn apply(
    state: &SharedState,
    user_id: Uuid,
    report: &PlaybackProgress,
) -> Result<ProgressAck, PlaybackError> {
    report
        .validate()
        .map_err(|err| PlaybackError(StatusCode::BAD_REQUEST, err.code()))?;
    let update = db::ProgressUpdate {
        position: report.clamped_position(),
        duration: report.duration_seconds,
        playing: report.counts_as_watching(),
        ends_session: report.ends_session(),
        played_to_end: report.played_to_end(),
        max_counted_gap: lisek_telemetry::MAX_COUNTED_GAP_SECONDS,
        stalls: report.stalls.min(i32::MAX as u32) as i32,
        stall_seconds: report.stall_seconds,
        dropped_frames: report.dropped_frames.map(|v| v.min(i32::MAX as u32) as i32),
        bandwidth_kbps: report.bandwidth_kbps.map(|v| v.min(i32::MAX as u32) as i32),
    };
    let outcome = db::record_progress(&pool(state).await?, user_id, report.session_id, &update)
        .await?
        .ok_or(PlaybackError(StatusCode::NOT_FOUND, "playback_not_found"))?;
    let session_id = report.session_id.to_string();
    if report.ends_session() {
        session::stop(&session_id).await;
    } else {
        session::touch(&session_id);
    }
    Ok(ProgressAck {
        resume_position_seconds: outcome.resume_position,
        watched: outcome.watched,
        next_report_seconds: lisek_telemetry::next_report_seconds(!update.playing),
    })
}

#[utoipa::path(
    post,
    path = "/api/telemetry/playback-progress",
    request_body = PlaybackProgress,
    responses(
        (status = 200, body = ProgressAck, description = "Stored; when to report next"),
        (status = 400, description = "invalid_progress / invalid_quality"),
        (status = 401),
        (status = 404, description = "The session is not the caller's, has ended or expired")
    ),
    security(("lisek_auth" = []))
)]
pub async fn playback_progress(
    auth: AuthUser,
    State(state): State<SharedState>,
    Json(report): Json<PlaybackProgress>,
) -> Result<Json<ProgressAck>, PlaybackError> {
    Ok(Json(apply(&state, auth.user.id, &report).await?))
}

/// The older per-session endpoint, kept for clients built against it.
#[derive(Deserialize, utoipa::ToSchema)]
pub struct ProgressRequest {
    position_seconds: f64,
    duration_seconds: f64,
    paused: bool,
    /// The viewer left (not: played to the end).
    #[serde(default)]
    ended: bool,
}

#[utoipa::path(put, path = "/api/playback/{session_id}/progress", params(("session_id" = Uuid, Path)), request_body = ProgressRequest, responses((status = 204), (status = 400), (status = 401), (status = 404)), security(("lisek_auth" = [])))]
pub async fn legacy_progress(
    auth: AuthUser,
    State(state): State<SharedState>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<ProgressRequest>,
) -> Result<StatusCode, PlaybackError> {
    let report = PlaybackProgress {
        session_id: id,
        position_seconds: req.position_seconds,
        duration_seconds: req.duration_seconds,
        paused: req.paused,
        event: if req.ended {
            ProgressEvent::Stop
        } else {
            ProgressEvent::Heartbeat
        },
        stalls: 0,
        stall_seconds: 0.0,
        dropped_frames: None,
        bandwidth_kbps: None,
        buffered_seconds: None,
    };
    apply(&state, auth.user.id, &report).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct RecentQuery {
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    50
}

#[utoipa::path(get, path = "/api/admin/stats/playback", params(("limit" = Option<i64>, Query, description = "1-500, default 50")), responses((status = 200, body = Vec<db::SessionQuality>), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn recent_sessions(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Query(query): Query<RecentQuery>,
) -> Result<Json<Vec<db::SessionQuality>>, PlaybackError> {
    let limit = query.limit.clamp(1, 500);
    Ok(Json(
        db::recent_sessions(&pool(&state).await?, limit).await?,
    ))
}
