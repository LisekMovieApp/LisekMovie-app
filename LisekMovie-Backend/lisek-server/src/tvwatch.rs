//! Faza 26: own TV channels — what a signed-in viewer sees: the channel
//! list with what is on now and next, the programme guide, a channel's
//! logo, and the stream itself (a ticket in the path, as in Faza 20).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lisek_db::tv as db;

use crate::auth::AuthUser;
use crate::state::SharedState;
use crate::tv::{TvError, pool};
use crate::tvlive::{StreamError, ensure_own};

#[derive(Serialize, utoipa::ToSchema)]
pub struct NowNext {
    pub title: String,
    pub subtitle: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

impl From<&db::Programme> for NowNext {
    fn from(p: &db::Programme) -> Self {
        NowNext { title: p.title.clone(), subtitle: p.subtitle.clone(), starts_at: p.starts_at, ends_at: p.ends_at }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TvChannelView {
    pub id: Uuid,
    pub number: String,
    pub name: String,
    pub group_title: Option<String>,
    pub has_logo: bool,
    pub now: Option<NowNext>,
    pub next: Option<NowNext>,
}

/// The programme on at `at` and the one after it.
pub fn now_and_next(programmes: &[db::Programme], at: DateTime<Utc>) -> (Option<NowNext>, Option<NowNext>) {
    let current = programmes.iter().position(|p| p.starts_at <= at && p.ends_at > at);
    let next_index = current.map_or_else(|| programmes.iter().position(|p| p.starts_at > at), |i| (i + 1 < programmes.len()).then_some(i + 1));
    (current.map(|i| NowNext::from(&programmes[i])), next_index.map(|i| NowNext::from(&programmes[i])))
}

#[utoipa::path(get,path="/api/tv/channels",tag="media",responses((status=200,body=Vec<TvChannelView>),(status=401)),security(("lisek_auth"=[])))]
pub async fn channels(_auth: AuthUser, State(state): State<SharedState>) -> Result<Json<Vec<TvChannelView>>, TvError> {
    let pool = pool(&state)?;
    let now = Utc::now();
    let mut out = Vec::new();
    for channel in db::list_channels(&pool).await?.into_iter().filter(|c| c.enabled) {
        let programmes = db::guide(&pool, channel.id, now, now + chrono::Duration::hours(8)).await?;
        let (current, next) = now_and_next(&programmes, now);
        out.push(TvChannelView { id: channel.id, number: channel.number, name: channel.name, group_title: channel.group_title, has_logo: channel.has_logo, now: current, next });
    }
    Ok(Json(out))
}

#[derive(Deserialize, utoipa::IntoParams)]
pub struct GuideQuery {
    /// Start (RFC 3339); default: now.
    pub from: Option<DateTime<Utc>>,
    /// Hours to show (1-48); default 6.
    pub hours: Option<i64>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct GuideChannel {
    pub channel_id: Uuid,
    pub programmes: Vec<db::Programme>,
}

#[utoipa::path(get,path="/api/tv/guide",tag="media",params(GuideQuery),responses((status=200,body=Vec<GuideChannel>),(status=401)),security(("lisek_auth"=[])))]
pub async fn guide(_auth: AuthUser, State(state): State<SharedState>, Query(q): Query<GuideQuery>) -> Result<Json<Vec<GuideChannel>>, TvError> {
    let pool = pool(&state)?;
    let from = q.from.unwrap_or_else(Utc::now);
    let to = from + chrono::Duration::hours(q.hours.unwrap_or(6).clamp(1, 48));
    let mut out = Vec::new();
    for channel in db::list_channels(&pool).await?.into_iter().filter(|c| c.enabled) {
        out.push(GuideChannel { channel_id: channel.id, programmes: db::guide(&pool, channel.id, from, to).await? });
    }
    Ok(Json(out))
}

#[utoipa::path(get,path="/api/tv/channels/{id}/logo",tag="media",params(("id"=Uuid,Path)),responses((status=200,description="Image"),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn logo(_auth: AuthUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<Response, TvError> {
    let pool = pool(&state)?;
    let not_found = || TvError::new(StatusCode::NOT_FOUND, "channel_not_found");
    let path = db::logo_path(&pool, id).await?.ok_or_else(not_found)?;
    let bytes = tokio::fs::read(path).await.map_err(|_| not_found())?;
    Ok(crate::media::image_response(bytes, "image/png"))
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct TvPlayResponse {
    pub session_id: Uuid,
    /// Live HLS playlist (no headers needed).
    pub stream_url: String,
    pub channel_id: Uuid,
    pub name: String,
}

#[utoipa::path(post,path="/api/tv/channels/{id}/play",tag="media",params(("id"=Uuid,Path)),responses((status=200,body=TvPlayResponse),(status=401),(status=404),(status=429,description="tv_capacity_reached"),(status=502,description="tv_stream_unavailable"),(status=503)),security(("lisek_auth"=[])))]
pub async fn play(auth: AuthUser, State(state): State<SharedState>, Path(id): Path<Uuid>) -> Result<Json<TvPlayResponse>, TvError> {
    let pool = pool(&state)?;
    let channel = db::get_channel(&pool, id).await?.filter(|c| c.enabled).ok_or(TvError::new(StatusCode::NOT_FOUND, "channel_not_found"))?;
    ensure_own(&state, &channel).await.map_err(|e| match e {
        StreamError::Capacity => TvError::new(StatusCode::TOO_MANY_REQUESTS, "tv_capacity_reached"),
        StreamError::Unavailable => TvError::new(StatusCode::BAD_GATEWAY, "tv_stream_unavailable"),
    })?;
    let (session_id, ticket) = state.tv.add_viewer(auth.user.id, id);
    tracing::info!(channel = %channel.name, user = %auth.user.username, "TV channel opened");
    Ok(Json(TvPlayResponse { session_id, stream_url: format!("/api/tv/stream/{session_id}/{ticket}/index.m3u8"), channel_id: id, name: channel.name }))
}

#[utoipa::path(delete,path="/api/tv/sessions/{session_id}",tag="media",params(("session_id"=Uuid,Path)),responses((status=204),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn stop(auth: AuthUser, State(state): State<SharedState>, Path(session_id): Path<Uuid>) -> StatusCode {
    if state.tv.stop_viewer(session_id, auth.user.id) { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND }
}

/// Only the playlist and the segments the segmenter writes.
pub fn valid_file(file: &str) -> bool {
    file == "index.m3u8" || (file.starts_with("seg") && file.ends_with(".ts") && file.len() > 6 && file.len() <= 16 && file[3..file.len() - 3].chars().all(|c| c.is_ascii_digit()))
}

pub async fn read_file(dir: &std::path::Path, file: &str) -> Response {
    let Ok(bytes) = tokio::fs::read(dir.join(file)).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = if file.ends_with(".m3u8") { "application/vnd.apple.mpegurl" } else { "video/mp2t" };
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "no-store")], bytes).into_response()
}

#[utoipa::path(get,path="/api/tv/stream/{session_id}/{ticket}/{file}",tag="media",params(("session_id"=Uuid,Path),("ticket"=String,Path),("file"=String,Path)),responses((status=200,description="HLS playlist or MPEG-TS segment"),(status=403),(status=404)))]
pub async fn stream(State(state): State<SharedState>, Path((session_id, ticket, file)): Path<(Uuid, String, String)>) -> Response {
    let Some(dir) = valid_file(&file).then(|| state.tv.touch_viewer(session_id, &ticket)).flatten() else {
        return StatusCode::FORBIDDEN.into_response();
    };
    read_file(&dir, &file).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn programme(start: i64, end: i64, title: &str) -> db::Programme {
        let base = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap();
        db::Programme {
            starts_at: base + chrono::Duration::minutes(start),
            ends_at: base + chrono::Duration::minutes(end),
            title: title.into(),
            subtitle: None,
            media_item_id: None,
            description: None,
        }
    }

    #[test]
    fn now_and_next_are_found_by_time() {
        let base = DateTime::<Utc>::from_timestamp(1_800_000_000, 0).unwrap();
        let list = vec![programme(0, 30, "A"), programme(30, 90, "B"), programme(90, 120, "C")];
        let at = |m| base + chrono::Duration::minutes(m);
        let (now, next) = now_and_next(&list, at(45));
        assert_eq!((now.unwrap().title, next.unwrap().title), ("B".into(), "C".into()));
        let (now, next) = now_and_next(&list, at(100));
        assert_eq!((now.unwrap().title, next.is_none()), ("C".into(), true));
        let (now, next) = now_and_next(&list, at(-10));
        assert_eq!((now.is_none(), next.unwrap().title), (true, "A".into()));
        assert!(now_and_next(&[], at(0)).0.is_none());
    }

    #[test]
    fn only_the_playlist_and_segments_are_served() {
        for ok in ["index.m3u8", "seg000001.ts", "seg1.ts"] {
            assert!(valid_file(ok), "{ok}");
        }
        for bad in ["../index.m3u8", "seg.ts", "segx.ts", "seg0000000000000.ts", "index.m3u8/x", "a.ts"] {
            assert!(!valid_file(bad), "{bad}");
        }
    }
}
