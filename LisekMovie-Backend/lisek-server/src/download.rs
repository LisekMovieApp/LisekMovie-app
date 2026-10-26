//! Downloads (offline viewing): the original file of a film or an
//! episode, as an attachment with a readable name (`Tytul_S01E01.mkv`).
//!
//! A browser download cannot send the `Authorization` header, and a
//! session token in a URL would end up in history and logs. So:
//!
//! 1. `POST /api/media/{id}/download-ticket` (signed in) checks the
//!    account may see the item and may download (`can_download`,
//!    parental controls), and returns a URL with a random ticket.
//! 2. `GET /api/media/{id}/download?ticket=...` serves the file. The ticket
//!    is bound to that account and item and lasts [`TICKET_TTL`], long
//!    enough for a big file and for resuming (HTTP Range) a broken
//!    download. The account's restrictions are checked again on every
//!    request.
//!
//! The original container is sent as is: it is what the library holds,
//! nothing is re-encoded and every player that downloads it (VLC, mpv,
//! the phone's own player) reads it.

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::playback::{PlaybackError, checked_file, pool};
use crate::state::SharedState;

/// How long a download URL works.
pub const TICKET_TTL: Duration = Duration::from_secs(12 * 60 * 60);
/// Live tickets kept at most (oldest dropped first beyond that).
const MAX_TICKETS: usize = 10_000;

#[derive(Debug, Clone)]
pub struct DownloadTicket {
    user_id: Uuid,
    item_id: Uuid,
    expires: Instant,
}

/// Issued tickets, in memory: a restart invalidates them (the app asks
/// for a new one with the next click).
#[derive(Debug, Default)]
pub struct DownloadTickets(std::sync::Mutex<HashMap<String, DownloadTicket>>);

impl DownloadTickets {
    fn issue(&self, user_id: Uuid, item_id: Uuid) -> String {
        let ticket = lisek_auth::generate_session_token();
        let now = Instant::now();
        if let Ok(mut map) = self.0.lock() {
            map.retain(|_, t| t.expires > now);
            if map.len() >= MAX_TICKETS
                && let Some(oldest) = map
                    .iter()
                    .min_by_key(|(_, t)| t.expires)
                    .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
            map.insert(
                ticket.clone(),
                DownloadTicket {
                    user_id,
                    item_id,
                    expires: now + TICKET_TTL,
                },
            );
        }
        ticket
    }

    /// The account a live ticket for `item_id` belongs to.
    fn check(&self, ticket: &str, item_id: Uuid) -> Option<Uuid> {
        let map = self.0.lock().ok()?;
        let found = map.get(ticket)?;
        (found.item_id == item_id && found.expires > Instant::now()).then_some(found.user_id)
    }
}

/// What the signed-in account may do beyond browsing.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MyAccessResponse {
    /// Downloads are allowed (the "Download" button shows).
    pub can_download: bool,
    /// Parental controls limit what this account sees.
    pub restricted: bool,
}

#[utoipa::path(get,path="/api/me/access",tag="auth",responses((status=200,body=MyAccessResponse),(status=401)),security(("lisek_auth"=[])))]
pub async fn my_access(auth: AuthUser) -> Json<MyAccessResponse> {
    Json(MyAccessResponse {
        can_download: auth.access.can_download,
        restricted: !auth.access.browses_everything(),
    })
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DownloadTicketResponse {
    /// Open this URL (no header needed) to download.
    pub url: String,
    pub file_name: String,
    pub size_bytes: u64,
    /// Seconds the URL keeps working.
    pub expires_in: u64,
}

/// A download URL for an item's original file.
#[utoipa::path(post,path="/api/media/{item_id}/download-ticket",tag="media",params(("item_id"=Uuid,Path)),responses((status=200,body=DownloadTicketResponse),(status=401),(status=403,description="downloads_disabled"),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn ticket(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<DownloadTicketResponse>, PlaybackError> {
    let pool = pool(&state).await?;
    if !auth.can_see(&pool, item_id).await? {
        return Err(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"));
    }
    if !auth.access.can_download {
        return Err(PlaybackError(StatusCode::FORBIDDEN, "downloads_disabled"));
    }
    let (path, name) = resolve(&pool, item_id).await?;
    let size_bytes = tokio::fs::metadata(&path)
        .await
        .map(|m| m.len())
        .map_err(|_| PlaybackError(StatusCode::NOT_FOUND, "media_file_unavailable"))?;
    let ticket = state.download_tickets.issue(auth.user.id, item_id);
    Ok(Json(DownloadTicketResponse {
        url: format!("/api/media/{item_id}/download?ticket={ticket}"),
        file_name: name,
        size_bytes,
        expires_in: TICKET_TTL.as_secs(),
    }))
}

/// The checked file path and the download name of an item.
async fn resolve(
    pool: &lisek_db::PgPool,
    item_id: Uuid,
) -> Result<(std::path::PathBuf, String), PlaybackError> {
    let not_found = || PlaybackError(StatusCode::NOT_FOUND, "media_not_found");
    let source = lisek_db::playback::source(pool, item_id)
        .await?
        .ok_or_else(not_found)?;
    let item = lisek_db::get_media_item_detail(pool, item_id)
        .await?
        .ok_or_else(not_found)?;
    let path = checked_file(&source).await?;
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let end_episode = lisek_db::episodes::end_episode(pool, item_id).await?;
    let name = download_name(
        &item.title,
        item.year,
        item.season.zip(item.episode),
        end_episode,
        &extension,
    );
    Ok((path, name))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DownloadQuery {
    /// From `POST /api/media/{item_id}/download-ticket`.
    pub ticket: String,
}

/// The original file as an attachment; supports HTTP Range (resume).
#[utoipa::path(get,path="/api/media/{item_id}/download",tag="media",params(("item_id"=Uuid,Path),DownloadQuery),responses((status=200,description="The file"),(status=206,description="Part of the file (Range)"),(status=401,description="invalid_ticket"),(status=403),(status=404),(status=416)))]
pub async fn download(
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
    Query(query): Query<DownloadQuery>,
    headers: HeaderMap,
) -> Result<Response, PlaybackError> {
    let user_id = state
        .download_tickets
        .check(&query.ticket, item_id)
        .ok_or(PlaybackError(StatusCode::UNAUTHORIZED, "invalid_ticket"))?;
    let pool = pool(&state).await?;
    // Restrictions may have changed since the ticket was issued.
    let (is_admin, access) = lisek_db::access::get_restrictions(&pool, user_id)
        .await?
        .ok_or(PlaybackError(StatusCode::UNAUTHORIZED, "invalid_ticket"))?;
    if !is_admin {
        if !access.can_download {
            return Err(PlaybackError(StatusCode::FORBIDDEN, "downloads_disabled"));
        }
        if !access.browses_everything()
            && lisek_db::access::item_visible(&pool, item_id, &access).await? != Some(true)
        {
            return Err(PlaybackError(StatusCode::NOT_FOUND, "media_not_found"));
        }
    }
    let (path, name) = resolve(&pool, item_id).await?;
    serve(&path, &name, &headers, user_id, item_id).await
}

async fn serve(
    path: &FsPath,
    name: &str,
    headers: &HeaderMap,
    user_id: Uuid,
    item_id: Uuid,
) -> Result<Response, PlaybackError> {
    let read_failed = || PlaybackError(StatusCode::INTERNAL_SERVER_ERROR, "media_read_failed");
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| PlaybackError(StatusCode::NOT_FOUND, "media_file_unavailable"))?;
    let length = file.metadata().await.map_err(|_| read_failed())?.len();
    let range = match headers.get(header::RANGE) {
        Some(value) => match value
            .to_str()
            .ok()
            .and_then(|value| lisek_playback::parse_range(value, length))
        {
            Some(range) => Some(range),
            None => {
                return Ok((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [(header::CONTENT_RANGE, format!("bytes */{length}"))],
                )
                    .into_response());
            }
        },
        None => None,
    };
    let (start, count) = range
        .map(|range| (range.start, range.end - range.start + 1))
        .unwrap_or((0, length));
    if start == 0 {
        tracing::info!(%user_id, %item_id, file = name, bytes = length, "download started");
    } else {
        tracing::debug!(%user_id, %item_id, file = name, start, "download resumed");
    }
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| read_failed())?;
    let mut builder = Response::builder()
        .status(if range.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, lisek_playback::content_type(path))
        .header(header::CONTENT_LENGTH, count.to_string())
        .header(header::CONTENT_DISPOSITION, content_disposition(name))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("Referrer-Policy", "no-referrer")
        .header("X-Content-Type-Options", "nosniff");
    if let Some(range) = range {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{length}", range.start, range.end),
        );
    }
    Ok(builder
        .body(Body::from_stream(ReaderStream::new(file.take(count))))
        .expect("static response headers"))
}

/// `Tytul_1977.mkv`, `Inazuma_Eleven_S01E05.mkv`, `Show_S01E01-E02.mkv`.
pub fn download_name(
    title: &str,
    year: Option<i32>,
    season_episode: Option<(i32, i32)>,
    end_episode: Option<i32>,
    extension: &str,
) -> String {
    let mut base: String = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    if base.is_empty() {
        base = "LisekMovie".to_string();
    }
    let base: String = base.chars().take(120).collect();
    let suffix = match (season_episode, end_episode) {
        (Some((s, e)), Some(end)) if end > e => format!("_S{s:02}E{e:02}-E{end:02}"),
        (Some((s, e)), _) => format!("_S{s:02}E{e:02}"),
        (None, _) => year.map(|y| format!("_{y}")).unwrap_or_default(),
    };
    let extension: String = extension
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    format!("{base}{suffix}.{extension}")
}

/// `attachment` with an ASCII fallback name and the exact UTF-8 one
/// (RFC 6266 / 5987), so "Gwiezdne_wojny" with Polish letters survives.
fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| match c {
            'ą' => 'a', 'ć' => 'c', 'ę' => 'e', 'ł' => 'l', 'ń' => 'n', 'ó' => 'o', 'ś' => 's',
            'ź' | 'ż' => 'z', 'Ą' => 'A', 'Ć' => 'C', 'Ę' => 'E', 'Ł' => 'L', 'Ń' => 'N',
            'Ó' => 'O', 'Ś' => 'S', 'Ź' | 'Ż' => 'Z',
            c if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') => c,
            _ => '_',
        })
        .collect();
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_well_and_stay_safe() {
        assert_eq!(download_name("Gwiezdne wojny: Część IV", Some(1977), None, None, "MKV"), "Gwiezdne_wojny_Część_IV_1977.mkv");
        assert_eq!(download_name("Inazuma Eleven", None, Some((1, 5)), None, "mkv"), "Inazuma_Eleven_S01E05.mkv");
        assert_eq!(download_name("Show", None, Some((2, 1)), Some(2), "mp4"), "Show_S02E01-E02.mp4");
        assert_eq!(download_name("../../etc/passwd", None, None, None, "m\"kv"), "etc_passwd.mkv");
        assert_eq!(download_name("???", None, None, None, "avi"), "LisekMovie.avi");
    }

    #[test]
    fn content_disposition_has_an_ascii_fallback_and_the_utf8_name() {
        assert_eq!(
            content_disposition("Część_1977.mkv"),
            "attachment; filename=\"Czesc_1977.mkv\"; filename*=UTF-8''Cz%C4%99%C5%9B%C4%87_1977.mkv"
        );
    }

    #[test]
    fn tickets_are_bound_to_one_item_and_expire() {
        let tickets = DownloadTickets::default();
        let user = Uuid::new_v4();
        let item = Uuid::new_v4();
        let ticket = tickets.issue(user, item);
        assert_eq!(tickets.check(&ticket, item), Some(user));
        assert_eq!(tickets.check(&ticket, Uuid::new_v4()), None);
        assert_eq!(tickets.check("guess", item), None);
        tickets.0.lock().unwrap().get_mut(&ticket).unwrap().expires = Instant::now();
        assert_eq!(tickets.check(&ticket, item), None);
    }
}
