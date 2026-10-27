//! Series pages (Faza 10): season cards, a season's episode list with each
//! viewer's progress, watched/favourite for a whole season, season posters
//! and episode thumbnails.
//!
//! Every route takes any episode id of the series (a series card's id is
//! its representative episode). Season posters are read where enrichment
//! writes them — `<series folder>/season01-poster.jpg`, else the local
//! cache — and fall back to the series poster. An episode thumbnail is the
//! provider's still (local cache), else a frame cut from the file on first
//! request (FFmpeg, shared limit with chapter thumbnails), else the
//! series backdrop.

use std::path::Path as FsPath;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use lisek_db::seasons as db;
use lisek_metadata::image_cache::{backdrop_cache_path, poster_cache_path};
use lisek_metadata::layout;
use lisek_playback::{chapters as thumbnails, transcode};

use crate::auth::AuthUser;
use crate::details::GENERATING;
use crate::media::{image_response, read_fresh_media_file, serve_item_image};
use crate::playback::{PlaybackError, checked_file, pool};
use crate::state::{SharedState, config_dir};

fn not_found() -> PlaybackError {
    PlaybackError(StatusCode::NOT_FOUND, "media_not_found")
}

async fn visible(
    state: &SharedState,
    auth: &AuthUser,
    item_id: Uuid,
) -> Result<lisek_db::PgPool, PlaybackError> {
    let pool = pool(state).await?;
    if !auth.can_see(&pool, item_id).await? {
        return Err(not_found());
    }
    Ok(pool)
}

fn season_poster_url(item_id: Uuid, season: i32) -> String {
    format!("/api/media/{item_id}/seasons/{season}/poster")
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SeasonCardResponse {
    pub season: i32,
    /// The provider's name ("Sezon 1", "Odcinki specjalne"); `None` before
    /// enrichment.
    pub name: Option<String>,
    pub episode_count: i64,
    pub unwatched_count: i64,
    /// Every episode is one of the viewer's favourites.
    pub favorite: bool,
    /// The episode "Play" opens: the first one not watched yet.
    pub play_id: Uuid,
    /// Always set: the season's own poster, else the series poster.
    pub poster_url: String,
    pub air_year: Option<i32>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SeasonCardsResponse {
    pub items: Vec<SeasonCardResponse>,
}

#[utoipa::path(get,path="/api/media/{item_id}/season-cards",tag="media",params(("item_id"=Uuid,Path)),responses((status=200,body=SeasonCardsResponse),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn season_cards(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<SeasonCardsResponse>, PlaybackError> {
    let pool = visible(&state, &auth, item_id).await?;
    let cards = db::season_cards(&pool, item_id, auth.user.id).await?;
    Ok(Json(SeasonCardsResponse {
        items: cards
            .into_iter()
            .map(|c| SeasonCardResponse {
                poster_url: season_poster_url(item_id, c.season),
                season: c.season,
                name: c.name,
                episode_count: c.episode_count,
                unwatched_count: c.unwatched_count,
                favorite: c.favorite,
                play_id: c.play_id,
                air_year: c.air_year,
            })
            .collect(),
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EpisodeRowResponse {
    pub id: Uuid,
    pub episode: i32,
    pub end_episode: Option<i32>,
    /// The provider's title; `None` before enrichment ("Odcinek N").
    pub title: Option<String>,
    pub overview: Option<String>,
    /// The file's length, else the provider's runtime.
    pub runtime_minutes: Option<u32>,
    pub rating: Option<f64>,
    pub air_date: Option<String>,
    /// `/api/media/{id}/thumb` (`?token=` works).
    pub thumb_url: String,
    pub watched: bool,
    pub favorite: bool,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct SeasonPageResponse {
    pub season: i32,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub air_year: Option<i32>,
    pub poster_url: String,
    pub episodes: Vec<EpisodeRowResponse>,
}

fn text(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn episode_row(row: db::EpisodeRow) -> EpisodeRowResponse {
    let data = row.data.unwrap_or(Value::Null);
    let file_minutes = row
        .duration_seconds
        .filter(|d| d.is_finite() && *d >= 60.0)
        .map(|d| (d / 60.0).round() as u32);
    let provider_minutes = data
        .get("runtime_minutes")
        .and_then(Value::as_u64)
        .and_then(|m| u32::try_from(m).ok())
        .filter(|m| *m > 0);
    EpisodeRowResponse {
        thumb_url: format!("/api/media/{}/thumb", row.id),
        id: row.id,
        episode: row.episode,
        end_episode: row.end_episode,
        title: text(&data, "title"),
        overview: text(&data, "overview"),
        runtime_minutes: file_minutes.or(provider_minutes),
        rating: data
            .get("rating")
            .and_then(Value::as_f64)
            .filter(|r| r.is_finite() && *r > 0.0 && *r <= 10.0),
        air_date: text(&data, "air_date"),
        watched: row.watched,
        favorite: row.favorite,
        position_seconds: row.position_seconds,
        duration_seconds: row.duration_seconds,
    }
}

#[utoipa::path(get,path="/api/media/{item_id}/seasons/{season}/episodes",tag="media",params(("item_id"=Uuid,Path),("season"=i32,Path)),responses((status=200,body=SeasonPageResponse),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn season_page(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path((item_id, season)): Path<(Uuid, i32)>,
) -> Result<Json<SeasonPageResponse>, PlaybackError> {
    let pool = visible(&state, &auth, item_id).await?;
    let page = db::season_page(&pool, item_id, season, auth.user.id)
        .await?
        .ok_or_else(not_found)?;
    Ok(Json(SeasonPageResponse {
        poster_url: season_poster_url(item_id, page.season),
        season: page.season,
        name: page.name,
        overview: page.overview,
        air_year: page.air_year,
        episodes: page.episodes.into_iter().map(episode_row).collect(),
    }))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SeasonFlagsRequest {
    pub watched: Option<bool>,
    pub favorite: Option<bool>,
}

/// Watched and/or favourite for every episode of the season (this viewer).
#[utoipa::path(put,path="/api/media/{item_id}/seasons/{season}/state",tag="media",params(("item_id"=Uuid,Path),("season"=i32,Path)),request_body=SeasonFlagsRequest,responses((status=204),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn set_season_state(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path((item_id, season)): Path<(Uuid, i32)>,
    Json(req): Json<SeasonFlagsRequest>,
) -> Result<StatusCode, PlaybackError> {
    let pool = visible(&state, &auth, item_id).await?;
    let changed = db::set_season_flags(
        &pool,
        auth.user.id,
        item_id,
        season,
        req.watched,
        req.favorite,
    )
    .await?;
    if changed == 0 {
        return Err(not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

/// A season's poster: `<series folder>/seasonNN-poster.jpg`, the local
/// cache, else the series poster.
#[utoipa::path(get,path="/api/media/{item_id}/seasons/{season}/poster",tag="media",params(("item_id"=Uuid,Path),("season"=i32,Path)),responses((status=200,description="JPEG"),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn season_poster(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path((item_id, season)): Path<(Uuid, i32)>,
) -> Result<Response, PlaybackError> {
    let pool = visible(&state, &auth, item_id).await?;
    if let Some((series_id, file)) = db::season_poster_source(&pool, item_id, season).await? {
        let name = layout::season_poster_name(season);
        let mut candidates = Vec::with_capacity(2);
        if let Some(dir) = layout::series_folder(FsPath::new(&file)) {
            candidates.push(dir.join(&name));
        }
        candidates.push(layout::season_poster_cache_path(
            &config_dir(),
            &series_id.to_string(),
            season,
        ));
        // A season poster in the series folder from before the last Fix
        // match belongs to the previous match.
        let reset_at = lisek_db::get_media_item_artwork_source(&pool, item_id)
            .await
            .ok()
            .flatten()
            .and_then(|(_, reset_at)| reset_at);
        for (i, path) in candidates.iter().enumerate() {
            let bytes = if i + 1 < candidates.len() {
                read_fresh_media_file(path, reset_at).await
            } else {
                tokio::fs::read(path).await.ok()
            };
            if let Some(bytes) = bytes {
                return Ok(image_response(bytes, "image/jpeg"));
            }
        }
    }
    Ok(serve_item_image(
        &state,
        item_id,
        "poster.jpg",
        poster_cache_path,
        "image/jpeg",
    )
    .await)
}

/// Where a generated frame is taken: a sixth into the episode (past the
/// opening titles), never later than 10 minutes.
fn frame_time(duration: Option<f64>) -> f64 {
    duration
        .filter(|d| d.is_finite() && *d > 0.0)
        .map(|d| (d / 6.0).min(600.0))
        .unwrap_or(30.0)
}

/// An episode's thumbnail (16:9): the provider's still, a frame cut from
/// the file, else the series backdrop.
#[utoipa::path(get,path="/api/media/{item_id}/thumb",tag="media",params(("item_id"=Uuid,Path)),responses((status=200,description="JPEG"),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn episode_thumb(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Response, PlaybackError> {
    let pool = visible(&state, &auth, item_id).await?;
    let key = item_id.to_string();
    let still = layout::episode_still_path(&config_dir(), &key);
    if let Ok(bytes) = tokio::fs::read(&still).await {
        return Ok(cached(bytes));
    }
    let frame = layout::episode_frame_path(&config_dir(), &key);
    if let Ok(bytes) = tokio::fs::read(&frame).await {
        return Ok(cached(bytes));
    }
    if let Some(bytes) = cut_frame(&pool, item_id, &frame).await {
        return Ok(cached(bytes));
    }
    Ok(serve_item_image(
        &state,
        item_id,
        "backdrop.jpg",
        backdrop_cache_path,
        "image/jpeg",
    )
    .await)
}

fn cached(bytes: Vec<u8>) -> Response {
    image_response(bytes, "image/jpeg")
}

/// One frame of the episode into `out` (FFmpeg, at most two at a time
/// with chapter thumbnails). `None` when the file, its probe or FFmpeg is
/// missing.
async fn cut_frame(pool: &lisek_db::PgPool, item_id: Uuid, out: &FsPath) -> Option<Vec<u8>> {
    let _permit = GENERATING.acquire().await.ok()?;
    if let Ok(bytes) = tokio::fs::read(out).await {
        return Some(bytes);
    }
    let source = lisek_db::playback::source(pool, item_id).await.ok()??;
    let input = checked_file(&source).await.ok()?;
    let (probe, _) = lisek_db::streams::stored_probe(pool, item_id)
        .await
        .ok()??;
    let video = probe.primary_video()?;
    let settings = lisek_db::playback::settings(pool).await.ok()?;
    let tools = transcode::tools(&settings).await;
    let at = frame_time(probe.duration_seconds);
    match thumbnails::generate(
        &settings,
        &tools,
        &input,
        video.index,
        at,
        video.color_transfer.as_deref(),
        out,
    )
    .await
    {
        Ok(()) => tokio::fs::read(out).await.ok(),
        Err(err) => {
            tracing::debug!(%item_id, %err, "episode frame failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_prefer_the_files_length_and_skip_blank_provider_values() {
        let row = db::EpisodeRow {
            id: Uuid::nil(),
            episode: 1,
            end_episode: None,
            data: Some(serde_json::json!({
                "title": "Najwyższy czas", "overview": " ", "runtime_minutes": 58,
                "rating": 8.3, "air_date": "2008-01-20"
            })),
            duration_seconds: Some(3061.0),
            watched: false,
            favorite: true,
            position_seconds: 0.0,
        };
        let out = episode_row(row);
        assert_eq!(out.title.as_deref(), Some("Najwyższy czas"));
        assert_eq!(out.overview, None);
        assert_eq!(out.runtime_minutes, Some(51));
        assert_eq!(out.rating, Some(8.3));
        assert_eq!(
            out.thumb_url,
            "/api/media/00000000-0000-0000-0000-000000000000/thumb"
        );
        let bare = episode_row(db::EpisodeRow {
            id: Uuid::nil(),
            episode: 2,
            end_episode: None,
            data: None,
            duration_seconds: None,
            watched: true,
            favorite: false,
            position_seconds: 0.0,
        });
        assert_eq!((bare.title, bare.runtime_minutes), (None, None));
    }

    #[test]
    fn frames_are_taken_past_the_opening() {
        assert_eq!(frame_time(Some(3000.0)), 500.0);
        assert_eq!(frame_time(Some(7200.0)), 600.0);
        assert_eq!(frame_time(None), 30.0);
    }
}
