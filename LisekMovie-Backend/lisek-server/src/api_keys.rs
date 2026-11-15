//! Faza 25: named API keys for scripts and automations — Discord and
//! Telegram bots, Home Assistant, own dashboards.
//!
//! - `GET/POST /api/admin/api-keys`, `PUT/DELETE /api/admin/api-keys/{id}`:
//!   the administrator lists, creates (the key is shown once, only its hash
//!   is stored), renames and revokes keys.
//! - [`ApiClient`]: a request carrying a key in `X-Lisek-Api-Key` (or the
//!   older `X-Lisek-Stats-Key`). A valid key marks the key as used
//!   (`last_used_at`, at most once a minute), so the panel shows which keys
//!   are still in use. Keys read what `StatsReader` lets them read today
//!   (`GET /api/stats/summary`); other endpoints can accept [`ApiClient`]
//!   the same way.

use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::{AdminUser, AuthError};
use crate::playback::PlaybackError;
use crate::state::SharedState;

/// The header a script sends its key in.
pub const API_KEY_HEADER: &str = "x-lisek-api-key";
/// The stats key's header before Faza 25; still accepted.
pub const LEGACY_STATS_KEY_HEADER: &str = "x-lisek-stats-key";
/// What every key starts with, so it is recognizable in a config file.
const KEY_PREFIX: &str = "lsk_";

/// A request authorized by an API key.
pub struct ApiClient {
    pub key_id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
}

impl ApiClient {
    pub fn has(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
}

/// Looks a key up. `None`: unknown.
pub async fn authenticate(state: &SharedState, key: &str) -> Result<ApiClient, AuthError> {
    let pool = state.pool().ok_or(AuthError::ServerNotReady)?;
    match lisek_db::api_keys::authenticate(&pool, &lisek_auth::hash_session_token(key)).await {
        Ok(Some((key_id, name, scopes))) => Ok(ApiClient { key_id, name, scopes }),
        Ok(None) => Err(AuthError::InvalidToken),
        Err(err) => {
            tracing::error!(%err, "API key lookup failed");
            Err(AuthError::Internal)
        }
    }
}

/// The key a request carries, if any.
pub fn presented_key(parts: &Parts) -> Option<String> {
    [API_KEY_HEADER, LEGACY_STATS_KEY_HEADER].iter().find_map(|header| {
        parts
            .headers
            .get(*header)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
    })
}

impl FromRequestParts<SharedState> for ApiClient {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, Self::Rejection> {
        let key = presented_key(parts).ok_or(AuthError::MissingToken)?;
        authenticate(state, &key).await
    }
}

fn pool(state: &SharedState) -> Result<lisek_db::PgPool, PlaybackError> {
    state
        .pool()
        .ok_or(PlaybackError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))
}

/// A key name: trimmed, 1–100 characters.
fn clean_name(name: &str) -> Result<String, PlaybackError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_key_name"));
    }
    Ok(name.to_owned())
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ApiKeyRequest {
    /// "Statystyki Discord", "Home Assistant" …
    pub name: String,
    /// `stats` and/or `iptv`. Missing: `stats` (a key made before scopes).
    #[serde(default)]
    pub scopes: Option<Vec<String>>,
}

/// Scopes checked: known, deduplicated, at least one.
fn clean_scopes(scopes: Option<Vec<String>>) -> Result<Vec<String>, PlaybackError> {
    let mut scopes = scopes.unwrap_or_else(|| vec!["stats".to_string()]);
    scopes.sort();
    scopes.dedup();
    if scopes.is_empty() || scopes.iter().any(|s| !lisek_db::api_keys::SCOPES.contains(&s.as_str())) {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_key_scopes"));
    }
    Ok(scopes)
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct NewApiKey {
    /// Shown once; only its hash is stored. Send it as `X-Lisek-Api-Key`.
    pub key: String,
    #[serde(flatten)]
    pub item: lisek_db::api_keys::ApiKey,
}

#[utoipa::path(get, path = "/api/admin/api-keys", tag = "admin", responses((status = 200, body = Vec<lisek_db::api_keys::ApiKey>), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn list(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<lisek_db::api_keys::ApiKey>>, PlaybackError> {
    Ok(Json(lisek_db::api_keys::list(&pool(&state)?).await?))
}

#[utoipa::path(post, path = "/api/admin/api-keys", tag = "admin", request_body = ApiKeyRequest, responses((status = 200, body = NewApiKey), (status = 400, description = "invalid_key_name"), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn create(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Json(req): Json<ApiKeyRequest>,
) -> Result<Json<NewApiKey>, PlaybackError> {
    let name = clean_name(&req.name)?;
    let scopes = clean_scopes(req.scopes)?;
    let key = format!("{KEY_PREFIX}{}", lisek_auth::generate_session_token());
    let prefix: String = key.chars().take(KEY_PREFIX.len() + 4).collect();
    let item = lisek_db::api_keys::create(
        &pool(&state)?,
        &name,
        &lisek_auth::hash_session_token(&key),
        &prefix,
        Some(auth.user.id),
        &scopes,
    )
    .await?;
    tracing::info!(key_id = %item.id, name, admin = %auth.user.username, "API key created");
    Ok(Json(NewApiKey { key, item }))
}

#[utoipa::path(put, path = "/api/admin/api-keys/{key_id}", tag = "admin", params(("key_id" = Uuid, Path)), request_body = ApiKeyRequest, responses((status = 204), (status = 400), (status = 401), (status = 403), (status = 404)), security(("lisek_auth" = [])))]
pub async fn rename(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Path(key_id): Path<Uuid>,
    Json(req): Json<ApiKeyRequest>,
) -> Result<StatusCode, PlaybackError> {
    let name = clean_name(&req.name)?;
    let scopes = req.scopes.map(|s| clean_scopes(Some(s))).transpose()?;
    let pool = pool(&state)?;
    if lisek_db::api_keys::rename(&pool, key_id, &name).await? {
        if let Some(scopes) = scopes {
            lisek_db::api_keys::set_scopes(&pool, key_id, &scopes).await?;
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(PlaybackError(StatusCode::NOT_FOUND, "api_key_not_found"))
    }
}

#[utoipa::path(delete, path = "/api/admin/api-keys/{key_id}", tag = "admin", params(("key_id" = Uuid, Path)), responses((status = 204), (status = 401), (status = 403), (status = 404)), security(("lisek_auth" = [])))]
pub async fn delete(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(key_id): Path<Uuid>,
) -> Result<StatusCode, PlaybackError> {
    if lisek_db::api_keys::delete(&pool(&state)?, key_id).await? {
        tracing::info!(%key_id, admin = %auth.user.username, "API key revoked");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(PlaybackError(StatusCode::NOT_FOUND, "api_key_not_found"))
    }
}
