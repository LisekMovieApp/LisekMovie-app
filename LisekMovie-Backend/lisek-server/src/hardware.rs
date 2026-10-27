//! Hardware diagnostics and GPU load (Faza 13), for the admin tab
//! "Sprzęt i Transkodowanie". The survey itself lives in
//! `lisek_playback::hardware`, the balancing in `lisek_playback::gpu`.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use lisek_core::playback_settings::HardwareDevice;
use lisek_playback::hardware::{self, BalancingInfo, HardwareReport};
use serde::{Deserialize, Serialize};

use crate::auth::AdminUser;
use crate::playback::{PlaybackError, pool};
use crate::state::SharedState;

/// Runs the hardware survey with the saved playback settings (FFmpeg
/// path, balancing) and returns it. Takes a few seconds.
#[utoipa::path(post, path = "/api/admin/hardware/diagnostics", responses((status = 200, body = HardwareReport), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn diagnostics(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<HardwareReport>, PlaybackError> {
    let settings = lisek_db::playback::settings(&pool(&state).await?).await?;
    Ok(Json(hardware::diagnose(&settings).await))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct HardwareStatus {
    /// The latest diagnostics since the server started, if any.
    pub report: Option<HardwareReport>,
    /// Live: devices transcodes are spread over and their load.
    pub balancing: BalancingInfo,
    /// The saved pool (Faza 14), also when no diagnostics ran yet.
    pub pool: Vec<HardwareDevice>,
}

#[utoipa::path(get, path = "/api/admin/hardware", responses((status = 200, body = HardwareStatus), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn status(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<HardwareStatus>, PlaybackError> {
    let settings = lisek_db::playback::settings(&pool(&state).await?).await?;
    Ok(Json(HardwareStatus {
        report: hardware::last_report(),
        balancing: hardware::balancing(&settings).await,
        pool: settings.hardware_pool,
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct PoolRequest {
    /// The whole pool as the admin panel shows it (enabled and disabled).
    pub devices: Vec<HardwareDevice>,
    /// Transcode with the pool (`hardware_acceleration = "pool"`). Off:
    /// back to the CPU when the pool was in use; other modes stay.
    pub use_pool: bool,
}

/// Saves the transcoding pool from the "Sprzęt i Transkodowanie" panel.
/// Running sessions keep their device; new ones use the new pool.
#[utoipa::path(put, path = "/api/admin/hardware/pool", request_body = PoolRequest, responses((status = 200, body = HardwareStatus), (status = 400, description = "invalid_pool"), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn set_pool(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Json(req): Json<PoolRequest>,
) -> Result<Json<HardwareStatus>, PlaybackError> {
    let pool = pool(&state).await?;
    let mut settings = lisek_db::playback::settings(&pool).await?;
    let mut ids = std::collections::HashSet::new();
    if !req.devices.iter().all(|d| ids.insert(d.id())) {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_pool"));
    }
    settings.hardware_pool = req.devices;
    if req.use_pool {
        settings.hardware_acceleration = "pool".into();
    } else if settings.hardware_acceleration == "pool" {
        settings.hardware_acceleration = "none".into();
    }
    if !settings.validate() {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_pool"));
    }
    lisek_db::playback::set_settings(&pool, &settings).await?;
    Ok(Json(HardwareStatus {
        report: hardware::last_report(),
        balancing: hardware::balancing(&settings).await,
        pool: settings.hardware_pool,
    }))
}
