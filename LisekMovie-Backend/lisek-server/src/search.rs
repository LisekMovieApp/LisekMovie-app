//! Global search (Faza 11): `GET /api/search?q=` — films, series,
//! episodes, people and collections in one answer, grouped for the
//! omnibox. Each group is one indexed query (`lisek_db::search`, trigram
//! indexes from migrations 0013/0023), the five run concurrently, and
//! every one applies the account's restrictions.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lisek_db::collections::CollectionSummary;
use lisek_db::search as db;

use crate::auth::AuthUser;
use crate::collections::card;
use crate::media::{MediaItemResponse, to_responses};
use crate::playback::{PlaybackError, pool};
use crate::state::SharedState;

/// Shorter queries match too much to be useful (and miss the trigram
/// indexes): the answer is empty.
pub const MIN_QUERY_CHARS: usize = 2;
const MAX_QUERY_CHARS: usize = 100;
/// Results per group by default, and at most.
const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 50;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SearchQuery {
    /// What to look for (2-100 characters; shorter gives empty groups).
    #[serde(default)]
    pub q: String,
    /// Results per group (default 20, at most 50).
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct EpisodeSearchResult {
    pub id: Uuid,
    pub series_title: String,
    pub season: i32,
    pub episode: i32,
    pub title: String,
    /// `/api/media/{id}/thumb` (16:9; `?token=` works).
    pub thumb_url: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct PersonSearchResult {
    pub person_id: Uuid,
    pub name: String,
    /// `"Actor"`, `"Director"` or `"Writer"`: what they are best known
    /// for here.
    pub role: String,
    pub character: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SearchResponse {
    /// The query as searched (trimmed).
    pub query: String,
    pub movies: Vec<MediaItemResponse>,
    pub series: Vec<MediaItemResponse>,
    pub episodes: Vec<EpisodeSearchResult>,
    pub people: Vec<PersonSearchResult>,
    /// Collection cards (`item_type` `"collection"`).
    pub collections: Vec<MediaItemResponse>,
}

impl SearchResponse {
    fn empty(query: String) -> Self {
        Self {
            query,
            movies: Vec::new(),
            series: Vec::new(),
            episodes: Vec::new(),
            people: Vec::new(),
            collections: Vec::new(),
        }
    }
}

#[utoipa::path(get,path="/api/search",tag="media",params(SearchQuery),responses((status=200,body=SearchResponse),(status=400),(status=401),(status=503)),security(("lisek_auth"=[])))]
pub async fn search(
    auth: AuthUser,
    State(state): State<SharedState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, PlaybackError> {
    let q = query.q.trim().to_owned();
    if q.chars().count() > MAX_QUERY_CHARS {
        return Err(PlaybackError(StatusCode::BAD_REQUEST, "invalid_query"));
    }
    if q.chars().count() < MIN_QUERY_CHARS {
        return Ok(Json(SearchResponse::empty(q)));
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let pool = pool(&state).await?;
    let access = &auth.access;
    let started = std::time::Instant::now();
    let (movies, series, episodes, people, collections) = tokio::try_join!(
        db::movies(&pool, &q, access, limit),
        db::series(&pool, &q, access, limit),
        db::episodes(&pool, &q, access, limit),
        db::people(&pool, &q, access, limit),
        db::collections(&pool, &q, access, limit),
    )?;
    tracing::debug!(
        query = q,
        movies = movies.len(),
        series = series.len(),
        episodes = episodes.len(),
        people = people.len(),
        collections = collections.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "search"
    );
    let (movies, series) = tokio::join!(to_responses(&pool, movies), to_responses(&pool, series));
    Ok(Json(SearchResponse {
        movies,
        series,
        episodes: episodes
            .into_iter()
            .map(|e| EpisodeSearchResult {
                thumb_url: format!("/api/media/{}/thumb", e.id),
                id: e.id,
                series_title: e.series_title,
                season: e.season,
                episode: e.episode,
                title: e.title,
            })
            .collect(),
        people: people
            .into_iter()
            .map(|p| PersonSearchResult {
                image_url: p.has_image.then(|| format!("/api/people/{}/image", p.id)),
                person_id: p.id,
                name: p.name,
                role: p.role,
                character: p.character,
            })
            .collect(),
        collections: collections
            .into_iter()
            .map(|c| {
                card(&CollectionSummary {
                    id: c.id,
                    name: c.name,
                    item_count: c.item_count,
                    first_year: c.first_year,
                    last_year: None,
                })
            })
            .collect(),
        query: q,
    }))
}
