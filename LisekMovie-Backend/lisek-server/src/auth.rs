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

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode, header, request::Parts};
use axum::response::{IntoResponse, Response};

use serde::{Deserialize, Serialize};

use lisek_core::User;
use lisek_db::access::Restrictions;

use crate::rate_limit;
use crate::state::SharedState;

/// A valid Argon2id hash of an arbitrary fixed password, computed once
/// offline — never a real credential. `login` verifies against this when
/// the requested username doesn't exist, so a nonexistent-user response and
/// a wrong-password response both pay the same Argon2 cost and take roughly
/// the same amount of time, instead of the former returning instantly and
/// handing an attacker a free username-enumeration oracle.
const DUMMY_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$/UJVqBKwAWpTqgeSjR05eg$DsvDwwPBraS50WB4uPz6ap96Q1M+ougojjJZvHCnRLY";

#[derive(Deserialize, utoipa::ToSchema)]
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

#[derive(Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub token: String,
    pub user: User,
}

/// Failed attempts are limited per client address and per username (see
/// `rate_limit`); a blocked client gets 429 with `Retry-After` before any
/// password hashing happens.
#[utoipa::path(post,path="/api/auth/login",tag="auth",request_body=LoginRequest,responses((status=200,body=LoginResponse),(status=400),(status=401),(status=429,description="Too many failed attempts; see Retry-After"),(status=500),(status=503)))]
pub async fn login(
    State(state): State<SharedState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<AuthReply, AuthError> {
    let pool = state.pool().ok_or(AuthError::ServerNotReady)?;

    if req.password.len()>1024 || req.username.len()>128 || req.username.trim().is_empty() { return Err(AuthError::InvalidCredentials); }
    // X-Forwarded-For is honoured only from the trusted proxies configured
    // for playback, so a client cannot pick its own rate-limit bucket.
    let trusted = lisek_db::playback::settings(&pool).await.map(|s| s.trusted_proxies).unwrap_or_default();
    let forwarded = headers.get("x-forwarded-for").and_then(|h| h.to_str().ok());
    let client = lisek_core::playback_settings::client_address(peer.ip(), forwarded, &trusted);
    let limit_keys = rate_limit::login_keys(client, &req.username);
    state.auth_failures.check(&limit_keys).map_err(AuthError::TooManyAttempts)?;

    let found = lisek_db::find_user_with_password_hash(&pool, req.username.trim()).await.map_err(|_| AuthError::Internal)?;
    let expected_hash = found.as_ref().map(|(_,hash)|hash.clone()).unwrap_or_else(||DUMMY_PASSWORD_HASH.to_owned());
    let check_hash=expected_hash.clone();
    let password=req.password;
    let password_ok=tokio::task::spawn_blocking(move ||lisek_auth::verify_password(&password,&check_hash).unwrap_or(false)).await.map_err(|_|AuthError::Internal)?;
    let Some((user,_))=found.filter(|(u,_)|password_ok && !u.disabled) else {
        state.auth_failures.record_failure(&limit_keys);
        tracing::warn!(%client, "failed login attempt");
        return Err(AuthError::InvalidCredentials);
    };
    let token=lisek_auth::generate_session_token();
    let created=lisek_db::users::login_session(&pool,user.id,&expected_hash,&lisek_auth::hash_session_token(&token),
        req.client.as_deref().unwrap_or("unknown"),req.device.as_deref().unwrap_or("unknown"),req.device_id.as_deref().unwrap_or("unknown"))
        .await.map_err(|_|AuthError::Internal)?;
    if !created{return Err(AuthError::InvalidCredentials);}

    tracing::info!(username = %user.username, "user logged in");

    Ok(no_store(LoginResponse { token, user }))
}

/// Also answers for a session that still has to change its temporary
/// password, so the client can show the password form.
#[utoipa::path(get,path="/api/auth/me",tag="auth",responses((status=200,body=User),(status=401),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn me(SessionUser { user, .. }: SessionUser) -> Json<User> {
    Json(user)
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LogoutResponse {
    pub ok: bool,
}

#[utoipa::path(post,path="/api/auth/logout",tag="auth",responses((status=200,body=LogoutResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn logout(
    SessionUser { token_hash, .. }: SessionUser,
    State(state): State<SharedState>,
) -> Result<Json<LogoutResponse>, AuthError> {
    let pool = state.pool().ok_or(AuthError::ServerNotReady)?;

    lisek_db::delete_session_by_token_hash(&pool, &token_hash)
        .await
        .map_err(|_| AuthError::Internal)?;

    Ok(Json(LogoutResponse { ok: true }))
}

/// The three access levels, as extractors. Taking one as a handler
/// parameter is how a handler requires it; the extractor rejects the
/// request before the handler body runs.
///
/// - [`SessionUser`]: any valid session, including one that still has to
///   replace its temporary password. Only `me`, `logout` and password
///   change accept this.
/// - [`AuthUser`]: a session with full access (password already changed).
/// - [`AdminUser`]: an [`AuthUser`] with the administrator role. Every
///   `/api/admin/*` route also sits behind this extractor as a router
///   layer (see `routes.rs`), so a handler that forgets it is still
///   protected.
///
/// The session is looked up once per request and cached in the request
/// extensions, so the router layer and the handler share one query.
pub struct SessionUser {
    pub user: User,
    pub(crate) token_hash: String,
}

pub struct AuthUser {
    pub user: User,
    pub(crate) token_hash: String,
    /// What this account may browse (parental controls). Always
    /// unrestricted for an administrator.
    pub access: Restrictions,
}

impl AuthUser {
    /// Whether this account may see `item_id`. An unrestricted account
    /// answers `true` without a query (the handler's own lookup still
    /// reports a missing item); a restricted one gets `false` for a
    /// missing item too.
    pub async fn can_see(
        &self,
        pool: &lisek_db::PgPool,
        item_id: uuid::Uuid,
    ) -> Result<bool, lisek_db::DbError> {
        if self.access.browses_everything() {
            return Ok(true);
        }
        Ok(lisek_db::access::item_visible(pool, item_id, &self.access)
            .await?
            .unwrap_or(false))
    }
}

pub struct AdminUser(pub AuthUser);

#[derive(Clone)]
struct ResolvedSession {
    user: User,
    token_hash: String,
    access: Restrictions,
}

async fn resolve_session(
    parts: &mut Parts,
    state: &SharedState,
) -> Result<ResolvedSession, AuthError> {
    if let Some(session) = parts.extensions.get::<ResolvedSession>() {
        return Ok(session.clone());
    }
    let token = token_from_header(parts)
        .or_else(|| is_image_path(parts.uri.path()).then(|| token_from_query(parts.uri.query())).flatten())
        .ok_or(AuthError::MissingToken)?;
    let token_hash = lisek_auth::hash_session_token(&token);

    let pool = state.pool().ok_or(AuthError::ServerNotReady)?;
    let (user, access) = lisek_db::access::find_session(&pool, &token_hash)
        .await
        .map_err(|_| AuthError::Internal)?
        .ok_or(AuthError::InvalidToken)?;

    let session = ResolvedSession {
        user,
        token_hash,
        access,
    };
    parts.extensions.insert(session.clone());
    Ok(session)
}

impl axum::extract::FromRequestParts<SharedState> for SessionUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, Self::Rejection> {
        let ResolvedSession { user, token_hash, .. } = resolve_session(parts, state).await?;
        Ok(SessionUser { user, token_hash })
    }
}

impl axum::extract::FromRequestParts<SharedState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, Self::Rejection> {
        let ResolvedSession {
            user,
            token_hash,
            access,
        } = resolve_session(parts, state).await?;
        if user.must_change_password {
            return Err(AuthError::PasswordChangeRequired);
        }
        Ok(AuthUser {
            user,
            token_hash,
            access,
        })
    }
}

impl axum::extract::FromRequestParts<SharedState> for AdminUser {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, Self::Rejection> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !user.user.is_admin {
            return Err(AuthError::AdminRequired);
        }
        Ok(AdminUser(user))
    }
}

/// Pulls the `Token=` value out of `LisekMovie Token=..., Client=..., ...`.
/// Permissive on purpose: pairs in any order, optional double-quotes around
/// a value, case-insensitive key names — real clients (and header-editing
/// humans debugging with curl) shouldn't have to match one exact layout.
fn parse_token(header_value: &str) -> Option<&str> {
    let rest = header_value.strip_prefix("LisekMovie ")?.trim_start();
    rest.split(',').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("token")
            .then(|| value.trim().trim_matches('"')).filter(|value| !value.is_empty())
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

fn is_image_path(path: &str) -> bool {
    let segments: Vec<_> = path.split('/').collect();
    let media = segments.len() >= 5 && segments[1] == "api" && segments[2] == "media";
    let collection = segments.len() == 5 && segments[1] == "api" && segments[2] == "collections";
    let person = segments.len() == 5 && segments[1] == "api" && segments[2] == "people";
    // `/api/media/{id}/poster|backdrop|logo`, the trickplay sprite
    // sheets `/api/media/{id}/trickplay/{n}` (CSS backgrounds), chapter
    // thumbnails `/api/media/{id}/chapters/{n}/image`,
    // `/api/collections/{id}/poster|backdrop` and `/api/people/{id}/image`.
    (media && segments.len() == 5 && matches!(segments[4], "poster" | "backdrop" | "logo"))
        || (media && segments.len() == 6 && segments[4] == "trickplay")
        || (media && segments.len() == 7 && segments[4] == "chapters" && segments[6] == "image")
        || (media && segments.len() == 7 && segments[4] == "seasons" && segments[6] == "poster")
        || (media && segments.len() == 5 && segments[4] == "thumb")
        || (collection && matches!(segments[4], "poster" | "backdrop"))
        || (person && segments[4] == "image")
}

#[derive(Debug)]
pub enum AuthError {
    PasswordChangeRequired,
    AdminRequired,
    /// Carries how long until the limit window ends.
    TooManyAttempts(std::time::Duration),
    MissingToken,
    InvalidToken,
    InvalidCredentials,
    ServerNotReady,
    Internal,
}

#[derive(Serialize, utoipa::ToSchema)]
struct AuthErrorBody {
    error_code: &'static str,
    message: &'static str,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            AuthError::TooManyAttempts(retry_after) => {
                let seconds = retry_after.as_secs().max(1).to_string();
                let body = AuthErrorBody {
                    error_code: "too_many_attempts",
                    message: "Too many failed sign-in attempts. Try again later.",
                };
                return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, seconds)], Json(body)).into_response();
            }
            AuthError::PasswordChangeRequired => (StatusCode::FORBIDDEN,"password_change_required","Set your own password before using the application."),
            AuthError::AdminRequired => (
                StatusCode::FORBIDDEN,
                "admin_required",
                "This account is not an administrator.",
            ),
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

pub type AuthReply = ([(axum::http::HeaderName,&'static str);1],Json<LoginResponse>);
pub fn no_store(body:LoginResponse)->AuthReply { ([(axum::http::header::CACHE_CONTROL,"no-store")],Json(body)) }

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

    #[test]
    fn query_tokens_only_for_images_and_trickplay_sheets() {
        let id = "/api/media/0b7d9c2e-0000-0000-0000-000000000000";
        assert!(is_image_path(&format!("{id}/poster")));
        assert!(is_image_path(&format!("{id}/trickplay/3")));
        assert!(!is_image_path(&format!("{id}/trickplay")), "the manifest is JSON, read with the header");
        assert!(is_image_path(&format!("{id}/chapters/4/image")));
        assert!(is_image_path(&format!("{id}/seasons/1/poster")));
        assert!(is_image_path(&format!("{id}/thumb")));
        assert!(!is_image_path(&format!("{id}/seasons/1/episodes")), "the episode list is JSON");
        assert!(!is_image_path(&format!("{id}/details")), "details are JSON");
        assert!(!is_image_path(&format!("{id}/next")));
        assert!(!is_image_path("/api/libraries"));
        let collection = "/api/collections/0b7d9c2e-0000-0000-0000-000000000000";
        assert!(is_image_path(&format!("{collection}/poster")));
        assert!(is_image_path(&format!("{collection}/backdrop")));
        assert!(!is_image_path(collection), "the collection itself is JSON");
        let person = "/api/people/0b7d9c2e-0000-0000-0000-000000000000";
        assert!(is_image_path(&format!("{person}/image")));
        assert!(!is_image_path(person), "the person page is JSON");
    }
}
