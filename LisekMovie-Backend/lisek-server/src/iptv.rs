//! Faza 26: the server as an IPTV provider — `/iptv/channels.m3u`,
//! `/iptv/epg.xml` and the channels' streams, for players such as VLC,
//! Kodi, Jellyfin or TiviMate.
//!
//! Every request carries an API key with the `iptv` scope: in the
//! `X-Lisek-Api-Key` header, or — because players take only a URL — as
//! `?key=` in it. The query form is accepted on `/iptv/*` and nowhere
//! else, and Lisek never logs it: the request log below shows the query
//! with the key masked. (A reverse proxy in front of the server may log the
//! full URL; that is outside Lisek's reach.)
//!
//! Own channels and the Faza 20 channels are listed together; both are
//! served from the shared channel streams (`tvlive`), so a player opening
//! a channel somebody already watches costs nothing extra.

use std::fmt::Write;

use axum::Json;
use axum::extract::{FromRequestParts, Path, Query, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use uuid::Uuid;

use lisek_db::{livetv as live_db, tv as db};

use crate::auth::AuthError;
use crate::state::SharedState;
use crate::tvlive::{StreamError, ensure_iptv, ensure_own};
use crate::tvwatch::{read_file, valid_file};

/// Past and future the guide covers.
const GUIDE_PAST_HOURS: i64 = 2;
const GUIDE_FUTURE_HOURS: i64 = 48;

/// An IPTV client: an API key with the `iptv` scope.
pub struct IptvClient;

/// The key in a query string.
fn key_from_query(query: Option<&str>) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == "key" && !value.is_empty()).then(|| value.to_owned())
    })
}

impl FromRequestParts<SharedState> for IptvClient {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &SharedState) -> Result<Self, Self::Rejection> {
        let key = crate::api_keys::presented_key(parts).or_else(|| key_from_query(parts.uri.query())).ok_or(AuthError::MissingToken)?;
        let client = crate::api_keys::authenticate(state, &key).await?;
        if !client.has("iptv") {
            return Err(AuthError::InvalidToken);
        }
        Ok(IptvClient)
    }
}

/// A query string with the key's value hidden (for logs).
pub fn mask_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some(("key", _)) => "key=***".to_string(),
            _ => pair.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// The request log of the IPTV routes: path and masked query, never the key.
pub async fn log_request(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(mask_query).unwrap_or_default();
    let method = request.method().clone();
    let response = next.run(request).await;
    // Segments would flood the log; playlists, the guide and failures show.
    if !path.ends_with(".ts") || !response.status().is_success() {
        tracing::debug!(%method, %path, query = %query, status = response.status().as_u16(), "IPTV request");
    }
    response
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct KeyQuery {
    /// The API key, for players that can only open a URL.
    pub key: Option<String>,
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// An M3U attribute value: quotes would end it.
fn attr(text: &str) -> String {
    text.replace('"', "'").replace(['\n', '\r'], " ")
}

/// One channel of the export.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: Uuid,
    pub number: Option<String>,
    pub name: String,
    pub group: Option<String>,
    /// An address (absolute for the provider's logos) or a path of this server.
    pub logo: Option<String>,
    /// The logo is a file of this server (needs the key).
    pub logo_local: bool,
}

fn with_key(path: &str, key: Option<&str>) -> String {
    match key {
        Some(key) => format!("{path}?key={key}"),
        None => path.to_string(),
    }
}

/// The M3U playlist. `base` is `http://host:port` (or empty for relative
/// links); `key` is added to the links only when the client used the query form.
pub fn render_m3u(entries: &[Entry], base: &str, key: Option<&str>) -> String {
    let mut out = String::from("#EXTM3U");
    let _ = write!(out, " x-tvg-url=\"{}\"", with_key(&format!("{base}/iptv/epg.xml"), key));
    out.push('\n');
    for e in entries {
        let _ = write!(out, "#EXTINF:-1 tvg-id=\"{}\"", e.id);
        if let Some(number) = &e.number {
            let _ = write!(out, " tvg-chno=\"{}\"", attr(number));
        }
        let _ = write!(out, " tvg-name=\"{}\"", attr(&e.name));
        if let Some(logo) = &e.logo {
            let logo = if e.logo_local { with_key(&format!("{base}{logo}"), key) } else { logo.clone() };
            let _ = write!(out, " tvg-logo=\"{}\"", attr(&logo));
        }
        if let Some(group) = &e.group {
            let _ = write!(out, " group-title=\"{}\"", attr(group));
        }
        let _ = writeln!(out, ",{}", attr(&e.name));
        let _ = writeln!(out, "{}", with_key(&format!("{base}/iptv/stream/{}/index.m3u8", e.id), key));
    }
    out
}

/// One programme of the XMLTV guide.
pub struct Slot {
    pub channel: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
}

fn xmltv_time(at: DateTime<Utc>) -> String {
    at.format("%Y%m%d%H%M%S +0000").to_string()
}

pub fn render_xmltv(entries: &[Entry], slots: &[Slot], base: &str, key: Option<&str>) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tv generator-info-name=\"LisekMovie\">\n");
    for e in entries {
        let _ = write!(out, "<channel id=\"{}\"><display-name>{}</display-name>", e.id, xml(&e.name));
        if let Some(number) = &e.number {
            let _ = write!(out, "<display-name>{}</display-name>", xml(number));
        }
        if let Some(logo) = &e.logo {
            let logo = if e.logo_local { with_key(&format!("{base}{logo}"), key) } else { logo.clone() };
            let _ = write!(out, "<icon src=\"{}\"/>", xml(&logo));
        }
        out.push_str("</channel>\n");
    }
    for s in slots {
        let _ = write!(
            out,
            "<programme start=\"{}\" stop=\"{}\" channel=\"{}\"><title>{}</title>",
            xmltv_time(s.starts_at),
            xmltv_time(s.ends_at),
            s.channel,
            xml(&s.title)
        );
        if let Some(sub) = &s.subtitle {
            let _ = write!(out, "<sub-title>{}</sub-title>", xml(sub));
        }
        if let Some(desc) = &s.description {
            let _ = write!(out, "<desc>{}</desc>", xml(desc));
        }
        out.push_str("</programme>\n");
    }
    out.push_str("</tv>\n");
    out
}

/// Every playlist line that is a file gets the key (relative links only).
pub fn rewrite_playlist(playlist: &str, key: Option<&str>) -> String {
    let Some(key) = key else { return playlist.to_string() };
    playlist
        .lines()
        .map(|line| if line.is_empty() || line.starts_with('#') { line.to_string() } else { format!("{line}?key={key}") })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

/// `http://host` from the request (behind a proxy: its forwarded scheme).
fn base_url(headers: &HeaderMap) -> String {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()).filter(|h| !h.is_empty()) else {
        return String::new();
    };
    let scheme = headers.get("x-forwarded-proto").and_then(|h| h.to_str().ok()).filter(|p| matches!(*p, "http" | "https")).unwrap_or("http");
    format!("{scheme}://{host}")
}

fn problem(code: &'static str, status: StatusCode) -> Response {
    (status, Json(serde_json::json!({ "error_code": code }))).into_response()
}

async fn entries(pool: &lisek_db::PgPool) -> Result<Vec<Entry>, lisek_db::DbError> {
    let mut out = Vec::new();
    for c in db::list_channels(pool).await?.into_iter().filter(|c| c.enabled) {
        out.push(Entry {
            id: c.id,
            number: Some(c.number),
            name: c.name,
            group: c.group_title,
            logo: c.has_logo.then(|| format!("/iptv/logo/{}", c.id)),
            logo_local: true,
        });
    }
    for c in live_db::export_channels(pool).await? {
        out.push(Entry { id: c.id, number: c.number, name: c.name, group: c.group_title, logo: c.logo_url, logo_local: false });
    }
    Ok(out)
}

/// The channel list for IPTV players.
#[utoipa::path(get,path="/iptv/channels.m3u",tag="iptv",params(KeyQuery),responses((status=200,description="M3U playlist"),(status=401),(status=403)))]
pub async fn channels_m3u(State(state): State<SharedState>, headers: HeaderMap, Query(q): Query<KeyQuery>) -> Response {
    let Some(pool) = state.pool() else { return problem("server_not_ready", StatusCode::SERVICE_UNAVAILABLE) };
    let Ok(entries) = entries(&pool).await else { return problem("internal_error", StatusCode::INTERNAL_SERVER_ERROR) };
    let body = render_m3u(&entries, &base_url(&headers), q.key.as_deref());
    ([(header::CONTENT_TYPE, "audio/x-mpegurl; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], body).into_response()
}

/// The programme guide (XMLTV) of every channel in the list.
#[utoipa::path(get,path="/iptv/epg.xml",tag="iptv",params(KeyQuery),responses((status=200,description="XMLTV guide"),(status=401),(status=403)))]
pub async fn epg_xml(State(state): State<SharedState>, headers: HeaderMap, Query(q): Query<KeyQuery>) -> Response {
    let Some(pool) = state.pool() else { return problem("server_not_ready", StatusCode::SERVICE_UNAVAILABLE) };
    let Ok(entries) = entries(&pool).await else { return problem("internal_error", StatusCode::INTERNAL_SERVER_ERROR) };
    let now = Utc::now();
    let (from, to) = (now - Duration::hours(GUIDE_PAST_HOURS), now + Duration::hours(GUIDE_FUTURE_HOURS));
    let mut slots = Vec::new();
    let own: Vec<Uuid> = db::list_channels(&pool).await.map(|c| c.into_iter().filter(|c| c.enabled).map(|c| c.id).collect()).unwrap_or_default();
    for id in own {
        let Ok(programmes) = db::guide(&pool, id, from, to).await else { continue };
        slots.extend(programmes.into_iter().map(|p| Slot { channel: id, starts_at: p.starts_at, ends_at: p.ends_at, title: p.title, subtitle: p.subtitle, description: p.description }));
    }
    if let Ok(programs) = live_db::programs(&pool, None, from, to).await {
        let listed: std::collections::HashSet<Uuid> = entries.iter().map(|e| e.id).collect();
        slots.extend(programs.into_iter().filter(|p| listed.contains(&p.channel_id)).map(|p| Slot {
            channel: p.channel_id,
            starts_at: p.starts_at,
            ends_at: p.ends_at,
            title: p.title,
            subtitle: p.subtitle,
            description: p.description,
        }));
    }
    let body = render_xmltv(&entries, &slots, &base_url(&headers), q.key.as_deref());
    ([(header::CONTENT_TYPE, "application/xml; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], body).into_response()
}

/// An own channel's logo.
#[utoipa::path(get,path="/iptv/logo/{channel_id}",tag="iptv",params(("channel_id"=Uuid,Path),KeyQuery),responses((status=200,description="Image"),(status=401),(status=403),(status=404)))]
pub async fn logo(State(state): State<SharedState>, Path(channel_id): Path<Uuid>) -> Response {
    let Some(pool) = state.pool() else { return problem("server_not_ready", StatusCode::SERVICE_UNAVAILABLE) };
    let Ok(Some(path)) = db::logo_path(&pool, channel_id).await else { return StatusCode::NOT_FOUND.into_response() };
    match tokio::fs::read(path).await {
        Ok(bytes) => crate::media::image_response(bytes, "image/png"),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// A channel's playlist and segments. Asking for the playlist starts the
/// channel when nobody is watching it.
#[utoipa::path(get,path="/iptv/stream/{channel_id}/{file}",tag="iptv",params(("channel_id"=Uuid,Path),("file"=String,Path),KeyQuery),responses((status=200,description="HLS playlist or MPEG-TS segment"),(status=401),(status=403),(status=404),(status=429),(status=502)))]
pub async fn stream(State(state): State<SharedState>, Path((channel_id, file)): Path<(Uuid, String)>, Query(q): Query<KeyQuery>) -> Response {
    if !valid_file(&file) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(pool) = state.pool() else { return problem("server_not_ready", StatusCode::SERVICE_UNAVAILABLE) };
    let dir = if file == "index.m3u8" {
        let started = match db::get_channel(&pool, channel_id).await {
            Ok(Some(channel)) if channel.enabled => ensure_own(&state, &channel).await,
            Ok(_) => match live_db::channel_source(&pool, channel_id).await {
                Ok(Some(channel)) if crate::livetv_parse::allowed_stream_url(&channel.stream_url) => {
                    let started = ensure_iptv(&state, channel.id, &channel.name, &channel.stream_url, false).await;
                    let _ = live_db::record_outcome(&pool, channel.id, started.is_ok()).await;
                    started
                }
                Ok(_) => return StatusCode::NOT_FOUND.into_response(),
                Err(_) => return problem("internal_error", StatusCode::INTERNAL_SERVER_ERROR),
            },
            Err(_) => return problem("internal_error", StatusCode::INTERNAL_SERVER_ERROR),
        };
        match started {
            Ok(stream) => stream.dir.clone(),
            Err(StreamError::Capacity) => return problem("tv_capacity_reached", StatusCode::TOO_MANY_REQUESTS),
            Err(StreamError::Unavailable) => return problem("tv_stream_unavailable", StatusCode::BAD_GATEWAY),
        }
    } else {
        match state.tv.touch_channel(channel_id) {
            Some(dir) => dir,
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    state.tv.touch_channel(channel_id);
    if file != "index.m3u8" {
        return read_file(&dir, &file).await;
    }
    let Ok(text) = tokio::fs::read_to_string(dir.join(&file)).await else { return StatusCode::NOT_FOUND.into_response() };
    ([(header::CONTENT_TYPE, "application/vnd.apple.mpegurl"), (header::CACHE_CONTROL, "no-store")], rewrite_playlist(&text, q.key.as_deref())).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(logo_local: bool) -> Entry {
        Entry {
            id: Uuid::from_u128(7),
            number: Some("5.1".into()),
            name: "Kino \"Lisek\" & Co".into(),
            group: Some("Filmy".into()),
            logo: Some(if logo_local { "/iptv/logo/7".into() } else { "https://cdn.example/l.png".into() }),
            logo_local,
        }
    }

    #[test]
    fn the_key_in_a_query_is_masked_for_logs() {
        assert_eq!(mask_query("key=lsk_secret&x=1"), "key=***&x=1");
        assert_eq!(mask_query("a=1&key=lsk_secret"), "a=1&key=***");
        assert_eq!(mask_query("x=1"), "x=1");
        assert!(!mask_query("key=lsk_secret").contains("secret"));
    }

    #[test]
    fn the_key_is_read_from_the_query_only_under_its_name() {
        assert_eq!(key_from_query(Some("key=abc&x=1")).as_deref(), Some("abc"));
        assert_eq!(key_from_query(Some("token=abc")), None);
        assert_eq!(key_from_query(Some("key=")), None);
        assert_eq!(key_from_query(None), None);
    }

    #[test]
    fn the_playlist_lists_channels_with_links_a_player_can_open() {
        let m3u = render_m3u(&[entry(true), entry(false)], "http://lisek:8409", Some("lsk_k"));
        let lines: Vec<&str> = m3u.lines().collect();
        assert_eq!(lines[0], "#EXTM3U x-tvg-url=\"http://lisek:8409/iptv/epg.xml?key=lsk_k\"");
        assert!(lines[1].starts_with("#EXTINF:-1 tvg-id=\"00000000-0000-0000-0000-000000000007\" tvg-chno=\"5.1\" tvg-name=\"Kino 'Lisek' & Co\""));
        assert!(lines[1].contains("tvg-logo=\"http://lisek:8409/iptv/logo/7?key=lsk_k\""), "own logos need the key");
        assert!(lines[1].ends_with("group-title=\"Filmy\",Kino 'Lisek' & Co"));
        assert_eq!(lines[2], "http://lisek:8409/iptv/stream/00000000-0000-0000-0000-000000000007/index.m3u8?key=lsk_k");
        assert!(lines[3].contains("tvg-logo=\"https://cdn.example/l.png\""), "a provider's logo is left alone");
    }

    #[test]
    fn a_header_client_gets_links_without_the_key() {
        let m3u = render_m3u(&[entry(true)], "", None);
        assert!(!m3u.contains("key="));
        assert!(m3u.contains("\n/iptv/stream/"));
    }

    #[test]
    fn the_guide_is_valid_xmltv() {
        let at = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap();
        let slot = Slot { channel: Uuid::from_u128(7), starts_at: at, ends_at: at + Duration::minutes(90), title: "A < B".into(), subtitle: Some("S01E02".into()), description: None };
        let xml = render_xmltv(&[entry(false)], &[slot], "", None);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<display-name>Kino &quot;Lisek&quot; &amp; Co</display-name>"));
        assert!(xml.contains("<programme start=\"20270115080000 +0000\" stop=\"20270115093000 +0000\" channel=\"00000000-0000-0000-0000-000000000007\">"));
        assert!(xml.contains("<title>A &lt; B</title><sub-title>S01E02</sub-title>"));
        assert!(xml.trim_end().ends_with("</tv>"));
    }

    #[test]
    fn segments_in_a_playlist_carry_the_key_when_the_client_uses_the_query() {
        let playlist = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4.0,\nseg000001.ts\n#EXTINF:4.0,\nseg000002.ts\n";
        let with = rewrite_playlist(playlist, Some("lsk_k"));
        assert!(with.contains("seg000001.ts?key=lsk_k\n") && with.contains("seg000002.ts?key=lsk_k\n"));
        assert!(with.contains("#EXTINF:4.0,\n"), "tags are left alone");
        assert_eq!(rewrite_playlist(playlist, None), playlist);
    }

    #[test]
    fn links_follow_the_scheme_the_proxy_reports() {
        let mut headers = HeaderMap::new();
        assert_eq!(base_url(&headers), "");
        headers.insert(header::HOST, "tv.example:8409".parse().unwrap());
        assert_eq!(base_url(&headers), "http://tv.example:8409");
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        assert_eq!(base_url(&headers), "https://tv.example:8409");
        headers.insert("x-forwarded-proto", "javascript".parse().unwrap());
        assert_eq!(base_url(&headers), "http://tv.example:8409");
    }
}
