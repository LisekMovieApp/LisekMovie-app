//! Faza 19: the signed-in account's own profile (avatar, UI language),
//! presence for the admin user list, and the server's default language.
//!
//! Avatars are uploaded as the raw request body (`Content-Type: image/*`),
//! decoded (JPEG, PNG, WebP, GIF), cropped to a centred square, scaled to
//! `AVATAR_SIZE` and stored as JPEG in `<config>/avatars/<user id>.jpg`.
//! The account's `avatar_version` (upload time in ms) goes into the avatar
//! URL, so a new picture is a new URL and browsers never show a stale one.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lisek_core::User;
use lisek_db::profiles as db;

use crate::auth::{AdminUser, SessionUser};
use crate::state::SharedState;

/// Side of the stored square avatar, in pixels.
pub const AVATAR_SIZE: u32 = 256;
/// Largest accepted upload.
pub const MAX_AVATAR_BYTES: usize = 10 * 1024 * 1024;
/// Largest accepted source image side (decoding bomb guard).
const MAX_SOURCE_SIDE: u32 = 12_000;
const JPEG_QUALITY: u8 = 86;
/// Transparent pixels are flattened onto the app's surface colour.
const BACKGROUND: [u8; 3] = [23, 23, 23];

pub struct ProfileError(StatusCode, &'static str);
impl IntoResponse for ProfileError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error_code": self.1 }))).into_response()
    }
}
impl From<lisek_db::DbError> for ProfileError {
    fn from(e: lisek_db::DbError) -> Self {
        tracing::error!(error = %e, "profile operation failed");
        internal()
    }
}
fn internal() -> ProfileError {
    ProfileError(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}
fn pool(state: &SharedState) -> Result<lisek_db::PgPool, ProfileError> {
    state.pool().ok_or(ProfileError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))
}

/// The avatar URL for an account, or `None` without an avatar.
pub fn avatar_url(user_id: Uuid, version: Option<i64>) -> Option<String> {
    version.map(|v| format!("/api/users/{user_id}/avatar?v={v}"))
}

fn avatars_dir() -> PathBuf {
    crate::state::config_dir().join("avatars")
}
fn avatar_path(user_id: Uuid) -> PathBuf {
    avatars_dir().join(format!("{user_id}.jpg"))
}

/// The signed-in account as `/api/auth/me` returns it.
#[derive(Serialize, utoipa::ToSchema)]
pub struct Me {
    #[serde(flatten)]
    pub user: User,
    /// `/api/users/{id}/avatar?v=…`, or null without an avatar.
    pub avatar_url: Option<String>,
    /// The server's default language: metadata (descriptions, artwork,
    /// category names) is fetched in it, whatever the account's UI language.
    pub server_language: String,
}

pub async fn me_for(state: &SharedState, user: User) -> Result<Me, ProfileError> {
    let pool = pool(state)?;
    let version = db::avatar_version(&pool, user.id).await?.flatten();
    let server_language = db::default_language(&pool).await?;
    Ok(Me {
        avatar_url: avatar_url(user.id, version),
        user,
        server_language,
    })
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct SetLanguage {
    /// `en` or `pl`.
    ui_language: String,
}

/// Changes the account's own interface language.
#[utoipa::path(put,path="/api/users/me/language",tag="auth",request_body=SetLanguage,responses((status=200,body=Me),(status=400),(status=401)),security(("lisek_auth"=[])))]
pub async fn set_language(
    SessionUser { user, .. }: SessionUser,
    State(state): State<SharedState>,
    Json(req): Json<SetLanguage>,
) -> Result<Json<Me>, ProfileError> {
    if !matches!(req.ui_language.as_str(), "en" | "pl") {
        return Err(ProfileError(StatusCode::BAD_REQUEST, "invalid_language"));
    }
    let updated = db::set_ui_language(&pool(&state)?, user.id, &req.ui_language)
        .await?
        .ok_or(ProfileError(StatusCode::UNAUTHORIZED, "invalid_token"))?;
    Ok(Json(me_for(&state, updated).await?))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct AvatarResponse {
    /// Null after the avatar was removed.
    pub avatar_url: Option<String>,
}

/// Why an upload is not a usable picture.
#[derive(Debug, PartialEq, Eq)]
pub enum AvatarImageError {
    Unsupported,
    TooLarge,
}

/// Decodes `bytes`, crops the centred square, scales it to `AVATAR_SIZE`
/// and encodes it as JPEG.
pub fn process_avatar(bytes: &[u8]) -> Result<Vec<u8>, AvatarImageError> {
    use image::{ImageFormat, ImageReader, Limits, imageops::FilterType};
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| AvatarImageError::Unsupported)?;
    if !matches!(
        reader.format(),
        Some(ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP | ImageFormat::Gif)
    ) {
        return Err(AvatarImageError::Unsupported);
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => AvatarImageError::TooLarge,
        _ => AvatarImageError::Unsupported,
    })?;
    let square = decoded.resize_to_fill(AVATAR_SIZE, AVATAR_SIZE, FilterType::Lanczos3).to_rgba8();
    let mut rgb = image::RgbImage::new(AVATAR_SIZE, AVATAR_SIZE);
    for (out, px) in rgb.pixels_mut().zip(square.pixels()) {
        let alpha = u16::from(px[3]);
        for c in 0..3 {
            let blended = (u16::from(px[c]) * alpha + u16::from(BACKGROUND[c]) * (255 - alpha) + 127) / 255;
            out[c] = blended as u8;
        }
    }
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|_| AvatarImageError::Unsupported)?;
    Ok(encoded)
}

/// Uploads a new avatar: the raw image as the request body (JPEG, PNG,
/// WebP or GIF, up to 10 MB). Stored as a 256×256 JPEG.
#[utoipa::path(post,path="/api/users/me/avatar",tag="auth",request_body(content=Vec<u8>,content_type="image/*"),responses((status=200,body=AvatarResponse),(status=400,description="avatar_empty / avatar_invalid_image"),(status=401),(status=413,description="avatar_too_large")),security(("lisek_auth"=[])))]
pub async fn upload_avatar(
    SessionUser { user, .. }: SessionUser,
    State(state): State<SharedState>,
    body: Body,
) -> Result<Json<AvatarResponse>, ProfileError> {
    let too_large = ProfileError(StatusCode::PAYLOAD_TOO_LARGE, "avatar_too_large");
    let bytes = axum::body::to_bytes(body, MAX_AVATAR_BYTES).await.map_err(|_| too_large)?;
    if bytes.is_empty() {
        return Err(ProfileError(StatusCode::BAD_REQUEST, "avatar_empty"));
    }
    let jpeg = tokio::task::spawn_blocking(move || process_avatar(&bytes))
        .await
        .map_err(|_| internal())?
        .map_err(|e| match e {
            AvatarImageError::TooLarge => ProfileError(StatusCode::PAYLOAD_TOO_LARGE, "avatar_too_large"),
            AvatarImageError::Unsupported => ProfileError(StatusCode::BAD_REQUEST, "avatar_invalid_image"),
        })?;
    let pool = pool(&state)?;
    let dir = avatars_dir();
    let path = avatar_path(user.id);
    let temporary = dir.join(format!("{}.{}.tmp", user.id, Uuid::new_v4()));
    let written = async {
        tokio::fs::create_dir_all(&dir).await?;
        tokio::fs::write(&temporary, &jpeg).await?;
        tokio::fs::rename(&temporary, &path).await
    }
    .await;
    if let Err(e) = written {
        let _ = tokio::fs::remove_file(&temporary).await;
        tracing::error!(error = %e, path = %path.display(), "saving avatar failed");
        return Err(internal());
    }
    let version = Utc::now().timestamp_millis();
    db::set_avatar_version(&pool, user.id, Some(version)).await?;
    tracing::info!(user_id = %user.id, bytes = jpeg.len(), "avatar updated");
    Ok(Json(AvatarResponse { avatar_url: avatar_url(user.id, Some(version)) }))
}

/// Removes the account's avatar (the initial is shown again).
#[utoipa::path(delete,path="/api/users/me/avatar",tag="auth",responses((status=200,body=AvatarResponse),(status=401)),security(("lisek_auth"=[])))]
pub async fn delete_avatar(
    SessionUser { user, .. }: SessionUser,
    State(state): State<SharedState>,
) -> Result<Json<AvatarResponse>, ProfileError> {
    db::set_avatar_version(&pool(&state)?, user.id, None).await?;
    match tokio::fs::remove_file(avatar_path(user.id)).await {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => tracing::warn!(error = %e, user_id = %user.id, "removing avatar file failed"),
    }
    tracing::info!(user_id = %user.id, "avatar removed");
    Ok(Json(AvatarResponse { avatar_url: None }))
}

/// An account's avatar (JPEG). Any signed-in account may load it; image
/// tags may pass the session as `?token=`.
#[utoipa::path(get,path="/api/users/{id}/avatar",tag="auth",params(("id"=Uuid,Path)),responses((status=200,content_type="image/jpeg"),(status=304),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn get_avatar(
    _auth: SessionUser,
    State(state): State<SharedState>,
    Path(id): Path<Uuid>,
) -> Result<Response, ProfileError> {
    let not_found = || ProfileError(StatusCode::NOT_FOUND, "avatar_not_found");
    db::avatar_version(&pool(&state)?, id).await?.flatten().ok_or_else(not_found)?;
    let bytes = tokio::fs::read(avatar_path(id)).await.map_err(|_| not_found())?;
    Ok(crate::media::image_response(bytes, "image/jpeg"))
}

/// Keep-alive from an open app tab. The request itself marks the session
/// as seen (see `Presence`); nothing else happens.
#[utoipa::path(post,path="/api/users/me/heartbeat",tag="auth",responses((status=204),(status=401)),security(("lisek_auth"=[])))]
pub async fn heartbeat(_auth: SessionUser) -> StatusCode {
    StatusCode::NO_CONTENT
}

/// Throttles `sessions.last_seen_at` writes: at most one per session per
/// `TOUCH_INTERVAL`, done off the request path.
#[derive(Default)]
pub struct Presence {
    touched: Mutex<HashMap<String, Instant>>,
}

const TOUCH_INTERVAL: Duration = Duration::from_secs(60);
const MAX_TRACKED: usize = 10_000;

impl Presence {
    /// Whether this session is due for a write (and records it as done).
    pub fn due(&self, token_hash: &str) -> bool {
        let now = Instant::now();
        let mut touched = self.touched.lock().unwrap_or_else(|e| e.into_inner());
        if touched.get(token_hash).is_some_and(|at| now.duration_since(*at) < TOUCH_INTERVAL) {
            return false;
        }
        if touched.len() >= MAX_TRACKED {
            touched.retain(|_, at| now.duration_since(*at) < TOUCH_INTERVAL);
        }
        touched.insert(token_hash.to_owned(), now);
        true
    }

    pub fn touch(&self, pool: &lisek_db::PgPool, token_hash: &str) {
        if !self.due(token_hash) {
            return;
        }
        let pool = pool.clone();
        let token_hash = token_hash.to_owned();
        tokio::spawn(async move {
            if let Err(e) = db::touch_session(&pool, &token_hash).await {
                tracing::debug!(error = %e, "marking session as seen failed");
            }
        });
    }
}

/// One account in the admin user list.
#[derive(Serialize, utoipa::ToSchema)]
pub struct AdminUserEntry {
    #[serde(flatten)]
    pub user: User,
    pub avatar_url: Option<String>,
    /// A request within the last 3 minutes (an open app tab sends one
    /// every minute) or playback in progress.
    pub online: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    /// What the account is playing right now.
    pub now_playing: Option<String>,
}

/// Every account with avatar and online status.
#[utoipa::path(get,path="/api/admin/users",responses((status=200,body=Vec<AdminUserEntry>),(status=403)),security(("lisek_auth"=[])))]
pub async fn admin_list(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<AdminUserEntry>>, ProfileError> {
    let rows = db::list_with_presence(&pool(&state)?).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| AdminUserEntry {
                avatar_url: avatar_url(r.user.id, r.avatar_version),
                user: r.user,
                online: r.online,
                last_seen_at: r.last_seen_at,
                now_playing: r.now_playing,
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(img: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn avatars_become_square_jpegs() {
        let wide = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(800, 300, image::Rgb([200, 30, 30])));
        let jpeg = process_avatar(&encode(wide, image::ImageFormat::Png)).unwrap();
        let back = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).unwrap();
        assert_eq!((back.width(), back.height()), (AVATAR_SIZE, AVATAR_SIZE));
        let px = back.to_rgb8().get_pixel(128, 128).0;
        assert!(px[0] > 170 && px[1] < 70, "{px:?}");
    }

    #[test]
    fn transparency_is_flattened_onto_the_surface_colour() {
        let clear = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(64, 64, image::Rgba([255, 255, 255, 0])));
        let jpeg = process_avatar(&encode(clear, image::ImageFormat::Png)).unwrap();
        let px = image::load_from_memory(&jpeg).unwrap().to_rgb8().get_pixel(10, 10).0;
        assert!(px.iter().all(|c| c.abs_diff(23) <= 4), "{px:?}");
    }

    #[test]
    fn other_files_are_rejected() {
        assert_eq!(process_avatar(b"not an image at all"), Err(AvatarImageError::Unsupported));
        assert_eq!(process_avatar(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), Err(AvatarImageError::Unsupported));
        let png = encode(image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4)), image::ImageFormat::Png);
        assert_eq!(process_avatar(&png[..png.len() / 2]), Err(AvatarImageError::Unsupported));
    }

    #[test]
    fn presence_writes_at_most_once_a_minute_per_session() {
        let presence = Presence::default();
        assert!(presence.due("a"));
        assert!(!presence.due("a"));
        assert!(presence.due("b"));
    }

    #[test]
    fn avatar_urls_carry_the_version() {
        let id = Uuid::nil();
        assert_eq!(avatar_url(id, None), None);
        assert_eq!(avatar_url(id, Some(5)).unwrap(), format!("/api/users/{id}/avatar?v=5"));
    }
}
