//! Login, logout, "who am I", and the `AuthUser` extractor that every
//! protected handler in this crate uses to require a valid session.
//!
//! Transport: a custom `Authorization: LisekMovie Token=<token>` header
//! (optionally with `Client=`, `Device=`, `DeviceId=` alongside it) — the
//! project's chosen auth scheme. This is deliberately not a cookie or a
//! JWT: an opaque, server-side-revocable token works the same way for a
//! browser, a future mobile app, or a headless API client, and revoking one
//! session (or listing a user's active ones, later) is a plain DELETE/SELECT
//! against `sessions`, not a JWT-blocklist workaround. The web app itself
//! stores the token in a plain (non-HttpOnly) cookie purely as its own
//! browser-side transport — see the frontend's `+layout.server.ts` — so
//! that both server-side rendering and client-side calls can attach the
//! same header; that's a web-client implementation detail the Rust side
//! neither knows nor cares about.

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, request::Parts};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use lisek_core::User;

use crate::state::SharedState;

/// A valid Argon2id hash of an arbitrary fixed password, computed once
/// offline — never a real credential. `login` verifies against this when
/// the requested username doesn't exist, so a nonexistent-user response and
/// a wrong-password response both pay the same Argon2 cost and take roughly
/// the same amount of time, instead of the former returning instantly and
/// handing an attacker a free username-enumeration oracle.
const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$/UJVqBKwAWpTqgeSjR05eg$DsvDwwPBraS50WB4uPz6ap96Q1M+ougojjJZvHCnRLY";

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    /// Free-text identity for the `sessions` table (a future "your active
    /// sessions" screen), never validated against anything — absent fields
    /// just fall back to "unknown".
    pub client: Option<String>,
    pub device: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
}

pub async fn login(
    State(state): State<SharedState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AuthError> {
    let pool = state.pool().await.ok_or(AuthError::ServerNotReady)?;

    let found = lisek_db::find_user_with_password_hash(&pool, &req.username)
        .await
        .map_err(|_| AuthError::Internal)?;

    let (user, password_ok) = match found {
        Some((user, password_hash)) => {
            let ok = lisek_auth::verify_password(&req.password, &password_hash).unwrap_or(false);
            (Some(user), ok)
        }
        None => {
            // Same Argon2 cost as a real check, spent on a hash that can
            // never match — see DUMMY_PASSWORD_HASH's doc comment.
            let _ = lisek_auth::verify_password(&req.password, DUMMY_PASSWORD_HASH);
            (None, false)
        }
    };

    let Some(user) = user.filter(|_| password_ok) else {
        return Err(AuthError::InvalidCredentials);
    };

    let token = lisek_auth::generate_session_token();
    let token_hash = lisek_auth::hash_session_token(&token);
    let expires_at = Utc::now() + lisek_auth::SESSION_TTL;

    lisek_db::create_session(
        &pool,
        user.id,
        &token_hash,
        req.client.as_deref().unwrap_or("unknown"),
        req.device.as_deref().unwrap_or("unknown"),
        req.device_id.as_deref().unwrap_or("unknown"),
        expires_at,
    )
    .await
    .map_err(|_| AuthError::Internal)?;

    tracing::info!(username = %user.username, "user logged in");

    Ok(Json(LoginResponse { token, user }))
}

pub async fn me(AuthUser { user, .. }: AuthUser) -> Json<User> {
    Json(user)
}

#[derive(Serialize)]
pub struct LogoutResponse {
    pub ok: bool,
}

pub async fn logout(
    AuthUser { token_hash, .. }: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<LogoutResponse>, AuthError> {
    let pool = state.pool().await.ok_or(AuthError::ServerNotReady)?;

    lisek_db::delete_session_by_token_hash(&pool, &token_hash)
        .await
        .map_err(|_| AuthError::Internal)?;

    Ok(Json(LogoutResponse { ok: true }))
}

/// Extractor that resolves the `Authorization` header to a logged-in user,
/// or fails the whole request with [`AuthError`] before the handler body
/// ever runs. Using this as a handler parameter — `async fn h(auth: AuthUser,
/// ...)` — IS how a route requires authentication in this codebase; there's
/// no separate `middleware::from_fn` layer to remember to add. Add every new
/// data-serving route in `main.rs` to the protected group with `AuthUser` as
/// its first parameter, the same way `me`/`logout` above do it.
pub struct AuthUser {
    pub user: User,
    token_hash: String,
}

impl axum::extract::FromRequestParts<SharedState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let token = token_from_header(parts)
            .or_else(|| token_from_query(parts.uri.query()))
            .ok_or(AuthError::MissingToken)?;
        let token_hash = lisek_auth::hash_session_token(&token);

        let pool = state.pool().await.ok_or(AuthError::ServerNotReady)?;
        let user = lisek_db::find_user_by_session_token_hash(&pool, &token_hash)
            .await
            .map_err(|_| AuthError::Internal)?
            .ok_or(AuthError::InvalidToken)?;

        Ok(AuthUser { user, token_hash })
    }
}

/// Pulls the `Token=` value out of `LisekMovie Token=..., Client=..., ...`.
/// Permissive on purpose: pairs in any order, optional double-quotes around
/// a value, case-insensitive key names — real clients (and header-editing
/// humans debugging with curl) shouldn't have to match one exact layout.
fn parse_token(header_value: &str) -> Option<&str> {
    let rest = header_value.strip_prefix("LisekMovie")?.trim_start();
    rest.split(',').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("token")
            .then(|| value.trim().trim_matches('"'))
    })
}

/// The normal path: every JSON API call sends the token in `Authorization`
/// (see this module's own doc comment for why a header, not a cookie).
fn token_from_header(parts: &Parts) -> Option<String> {
    let header_value = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())?;
    parse_token(header_value).map(str::to_owned)
}

/// The one exception: an `<img src>` (or any other browser-native resource
/// fetch) can't attach a custom header, so the two image-serving routes in
/// `media.rs` are reachable this way too — `?token=...`, checked only once
/// the header above is absent, so every JSON endpoint's behavior is
/// unchanged. Safe to parse with a plain `&`/`=` split and no percent-
/// decoding: `lisek_auth::generate_session_token` only ever produces
/// URL-safe-base64 characters (`[A-Za-z0-9_-]`), none of which a query
/// string would ever need to escape.
fn token_from_query(query: Option<&str>) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "token" && !value.is_empty()).then(|| value.to_owned())
    })
}

#[derive(Debug)]
pub enum AuthError {
    MissingToken,
    InvalidToken,
    InvalidCredentials,
    ServerNotReady,
    Internal,
}

#[derive(Serialize)]
struct AuthErrorBody {
    error_code: &'static str,
    message: &'static str,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            AuthError::MissingToken => (
                StatusCode::UNAUTHORIZED,
                "missing_token",
                "Authentication is required.",
            ),
            AuthError::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "This session is invalid or has expired.",
            ),
            AuthError::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Incorrect username or password.",
            ),
            AuthError::ServerNotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "server_not_ready",
                "The server isn't fully configured yet.",
            ),
            AuthError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Something went wrong while checking your session.",
            ),
        };

        (
            status,
            Json(AuthErrorBody {
                error_code,
                message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_alone() {
        assert_eq!(parse_token("LisekMovie Token=abc123"), Some("abc123"));
    }

    #[test]
    fn parses_token_among_other_fields_in_any_order() {
        assert_eq!(
            parse_token("LisekMovie Client=web, DeviceId=xyz, Token=abc123, Device=Browser"),
            Some("abc123")
        );
    }

    #[test]
    fn accepts_quoted_values() {
        assert_eq!(
            parse_token(r#"LisekMovie Client="web", Token="abc123""#),
            Some("abc123")
        );
    }

    #[test]
    fn rejects_missing_token_or_wrong_scheme() {
        assert_eq!(parse_token("LisekMovie Client=web"), None);
        assert_eq!(parse_token("Bearer abc123"), None);
        assert_eq!(parse_token(""), None);
    }

    #[test]
    fn query_token_reads_the_token_param() {
        assert_eq!(
            token_from_query(Some("token=abc123")),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn query_token_finds_it_among_other_params_in_any_order() {
        assert_eq!(
            token_from_query(Some("size=w500&token=abc123&v=2")),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn query_token_rejects_missing_or_empty() {
        assert_eq!(token_from_query(None), None);
        assert_eq!(token_from_query(Some("")), None);
        assert_eq!(token_from_query(Some("size=w500")), None);
        assert_eq!(token_from_query(Some("token=")), None);
    }
}
