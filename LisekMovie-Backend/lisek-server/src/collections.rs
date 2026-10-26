//! Collections ("box sets"): films a metadata provider groups into one
//! series. Created during enrichment (`metadata_enrichment`), read here.
//! Every endpoint applies the account's restrictions: a collection shows
//! only the films the account may see, and one with none visible is a
//! 404.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use uuid::Uuid;

use lisek_db::collections::{self as db, CollectionSummary};

use crate::auth::AuthUser;
use crate::media::{
    LibraryRowResponse, MediaError, MediaItemResponse, ensure_visible, image_response,
    serve_item_image, to_responses, visible_library,
};
use crate::metadata_enrichment::collection_image_path;
use crate::state::{SharedState, config_dir};

/// Collection rows on a library page are short: a library rarely has more.
const LIBRARY_COLLECTION_LIMIT: i64 = 60;

/// A collection card in the shape every other card has: `item_type`
/// `"collection"`, artwork under `/api/collections/{id}/...`, `year` the
/// first film's year.
pub(crate) fn card(summary: &CollectionSummary) -> MediaItemResponse {
    MediaItemResponse::from(lisek_core::MediaItem {
        id: summary.id,
        library_id: Uuid::nil(),
        item_type: "collection".to_string(),
        title: summary.name.clone(),
        year: summary.first_year,
        overview: None,
        season: None,
        episode: None,
        created_at: chrono::Utc::now(),
    })
    .with_count(summary.item_count)
}

impl MediaItemResponse {
    fn with_count(mut self, count: i64) -> Self {
        self.item_count = Some(count);
        self
    }
}

/// Collections with two or more films in this library, by name — the
/// "Kolekcje" row of a library page.
#[utoipa::path(get,path="/api/libraries/{library_id}/collections",tag="media",params(("library_id"=Uuid,Path)),responses((status=200,body=LibraryRowResponse),(status=401),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn library_collections(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<LibraryRowResponse>, MediaError> {
    let pool = state.pool().ok_or(MediaError::ServerNotReady)?;
    let library = visible_library(&pool, &auth, library_id).await?;
    let collections =
        db::library_collections(&pool, library_id, &auth.access, LIBRARY_COLLECTION_LIMIT)
            .await
            .map_err(|_| MediaError::Internal)?;
    Ok(Json(LibraryRowResponse {
        library: library.into(),
        items: collections.iter().map(card).collect(),
    }))
}

/// One collection with its films in release order.
#[derive(Serialize, utoipa::ToSchema)]
pub struct CollectionResponse {
    pub id: Uuid,
    pub name: String,
    /// Films the account can see.
    pub item_count: i64,
    pub first_year: Option<i32>,
    pub last_year: Option<i32>,
    pub poster_url: String,
    pub backdrop_url: String,
    pub items: Vec<MediaItemResponse>,
}

async fn collection_response(
    pool: &lisek_db::PgPool,
    auth: &AuthUser,
    collection_id: Uuid,
) -> Result<Option<CollectionResponse>, MediaError> {
    let Some(summary) = db::get(pool, collection_id, &auth.access)
        .await
        .map_err(|_| MediaError::Internal)?
    else {
        return Ok(None);
    };
    let members = db::members(pool, collection_id, &auth.access)
        .await
        .map_err(|_| MediaError::Internal)?;
    Ok(Some(CollectionResponse {
        poster_url: format!("/api/collections/{}/poster", summary.id),
        backdrop_url: format!("/api/collections/{}/backdrop", summary.id),
        id: summary.id,
        name: summary.name,
        item_count: summary.item_count,
        first_year: summary.first_year,
        last_year: summary.last_year,
        items: to_responses(pool, members).await,
    }))
}

#[utoipa::path(get,path="/api/collections/{collection_id}",tag="media",params(("collection_id"=Uuid,Path)),responses((status=200,body=CollectionResponse),(status=401),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn get_collection(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(collection_id): Path<Uuid>,
) -> Result<Json<CollectionResponse>, MediaError> {
    let pool = state.pool().ok_or(MediaError::ServerNotReady)?;
    collection_response(&pool, &auth, collection_id)
        .await?
        .map(Json)
        .ok_or(MediaError::NotFound)
}

/// The collection a film belongs to, with its other films (the detail
/// page's "Z kolekcji" row). 204 when it belongs to none.
#[utoipa::path(get,path="/api/media/{item_id}/collection",tag="media",params(("item_id"=Uuid,Path)),responses((status=200,body=CollectionResponse),(status=204,description="Not part of a collection"),(status=401),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn item_collection(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Response, MediaError> {
    let pool = state.pool().ok_or(MediaError::ServerNotReady)?;
    ensure_visible(&pool, &auth, item_id).await?;
    let Some(collection_id) = db::of_item(&pool, item_id)
        .await
        .map_err(|_| MediaError::Internal)?
    else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    Ok(match collection_response(&pool, &auth, collection_id).await? {
        Some(collection) => Json(collection).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

/// The collection's own artwork, else its first visible film's.
async fn serve_collection_image(
    state: &SharedState,
    auth: &AuthUser,
    collection_id: Uuid,
    file_name: &str,
    item_file: &str,
    item_cache: fn(&std::path::Path, Uuid) -> std::path::PathBuf,
) -> Result<Response, MediaError> {
    let pool = state.pool().ok_or(MediaError::ServerNotReady)?;
    let stand_in = db::artwork_stand_in(&pool, collection_id, &auth.access)
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::NotFound)?;
    let own = collection_image_path(&config_dir(), collection_id, file_name);
    if let Ok(bytes) = tokio::fs::read(&own).await {
        return Ok(image_response(bytes, "image/jpeg"));
    }
    Ok(serve_item_image(state, stand_in, item_file, item_cache, "image/jpeg").await)
}

#[utoipa::path(get,path="/api/collections/{collection_id}/poster",tag="media",params(("collection_id"=Uuid,Path)),responses((status=200,description="Image bytes"),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn collection_poster(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(collection_id): Path<Uuid>,
) -> Result<Response, MediaError> {
    serve_collection_image(
        &state,
        &auth,
        collection_id,
        "poster.jpg",
        "poster.jpg",
        lisek_metadata::image_cache::poster_cache_path,
    )
    .await
}

#[utoipa::path(get,path="/api/collections/{collection_id}/backdrop",tag="media",params(("collection_id"=Uuid,Path)),responses((status=200,description="Image bytes"),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn collection_backdrop(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(collection_id): Path<Uuid>,
) -> Result<Response, MediaError> {
    serve_collection_image(
        &state,
        &auth,
        collection_id,
        "backdrop.jpg",
        "backdrop.jpg",
        lisek_metadata::image_cache::backdrop_cache_path,
    )
    .await
}
