//! Client-facing media endpoints: the home-page feed (hero item + four
//! rows), one item's full detail (the "Movie Detail View" the admin asked
//! for once cards became clickable), and serving one item's
//! poster/backdrop/logo image. No `require_admin` gate here — unlike
//! everything in `admin.rs`, browsing the library and viewing its images
//! and details is for every signed-in account, not just admins; `AuthUser`
//! alone (see its own module's doc comment on why every data-serving route
//! needs it) is this module's whole security boundary. The *editing*
//! surface for an item's metadata stays admin-only, unchanged, in
//! `admin.rs`'s own `get_media_item`/`update_media_item` — this module's
//! [`media_item_detail`] is a read-only, non-admin-safe view of the same
//! row (no `original_title`/`locked_fields`/`forced_provider` exposed here,
//! nothing an ordinary account shouldn't see).
//!
//! As of Faza 5 round 2, an item's image doesn't always live in the local
//! `config_dir` cache — a library with `save_metadata_in_media_folder`
//! turned on has it written directly next to the video file instead (see
//! `metadata_enrichment::enrich_item`). [`serve_item_image`] tries that
//! media-folder copy first (looking the item's `file_path` up fresh on
//! every request, rather than trusting a value cached at scan time — an
//! admin can toggle the setting or move files between scans) and only
//! falls back to the local cache when there's no media-folder copy to
//! read, which also transparently covers a library that has the setting
//! turned off, or a fallback-flagged item whose image only ever made it
//! into the cache in the first place.

use std::path::{Path as FsPath, PathBuf};

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::state::SharedState;

/// One item as the frontend consumes it: `lisek_core::MediaItem` plus its
/// image URLs, which aren't stored anywhere — they're just this item's id
/// slotted into the routes below. Always present, even for an item with no
/// cached image of that kind yet; the routes themselves 404 rather than
/// this struct trying to know in advance whether a file exists on disk —
/// mirrors the poster/backdrop `<img onerror>` pattern already established
/// on the frontend (see `MediaCard.svelte`/`HeroBanner.svelte`), which
/// `logo_url` reuses as-is: a card or the Detail View just tries loading it
/// and falls back to the plain text title if that 404s, exactly like a
/// missing poster falls back to nothing.
///
/// `item_type` is `"movie"` or `"series"` as of Runda 8 — never a bare
/// `"episode"` any more (see `lisek_db`'s `home_candidates` CTE doc
/// comment): every response this module hands out already represents
/// something browsable on its own, not one file out of a season.
#[derive(Serialize, utoipa::ToSchema)]
pub struct MediaItemResponse {
    pub id: Uuid,
    pub item_type: String,
    pub title: String,
    pub year: Option<i32>,
    pub overview: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub poster_url: String,
    pub backdrop_url: String,
    pub logo_url: String,
    /// `"new_series"` / `"new_season"` / `"new_episodes"` for a `"series"`
    /// row (see `lisek_db::classify_series_badge`'s doc comment), or
    /// `"new_movie"` for a `"movie"` row added within
    /// `lisek_db::RECENT_BADGE_DAYS` (Runda 9 — see [`is_recently_added`]),
    /// or `None`. Only ever set by [`to_response`], for whichever row it's
    /// given; the plain [`From`] impl below always leaves it `None`, which
    /// is what [`media_item_detail`] (the single-item Detail View endpoint)
    /// returns as-is — the badge is a browse-card affordance (home feed,
    /// and as of Runda 9 the `/library/[id]` grid too — both go through
    /// [`to_response`]/[`to_responses`]), not something the Detail View
    /// itself was ever asked to show.
    pub badge: Option<String>,
}

impl From<lisek_core::MediaItem> for MediaItemResponse {
    fn from(item: lisek_core::MediaItem) -> Self {
        Self {
            poster_url: format!("/api/media/{}/poster", item.id),
            backdrop_url: format!("/api/media/{}/backdrop", item.id),
            logo_url: format!("/api/media/{}/logo", item.id),
            id: item.id,
            item_type: item.item_type,
            title: item.title,
            year: item.year,
            overview: item.overview,
            season: item.season,
            episode: item.episode,
            badge: None,
        }
    }
}

/// [`MediaItemResponse::from`] plus this item's "new" badge, if any — a
/// separate async step because a `"series"` row's badge needs its own
/// database round trip (see `lisek_db::classify_series_badge`), which a
/// plain `From` impl can't do; a `"movie"` row's badge is decided purely
/// from data already in hand ([`is_recently_added`]), but is computed here
/// too rather than in the `From` impl, so both badge kinds live in exactly
/// one place instead of splitting "when does a card get a badge" across two
/// call sites. Used for every item the home feed hands back, and (Runda 9)
/// every item a `/library/[id]` grid hands back; [`media_item_detail`]
/// deliberately skips this and uses the plain `From` conversion instead
/// (see [`MediaItemResponse::badge`]'s own doc comment for why).
async fn to_response(pool: &lisek_db::PgPool, item: lisek_core::MediaItem) -> MediaItemResponse {
    let badge = match item.item_type.as_str() {
        "series" => lisek_db::classify_series_badge(pool, item.library_id, &item.title)
            .await
            .ok()
            .flatten()
            .map(|badge| badge.as_str().to_string()),
        "movie" if is_recently_added(item.created_at, Utc::now()) => Some("new_movie".to_string()),
        _ => None,
    };

    let mut response = MediaItemResponse::from(item);
    response.badge = badge;
    response
}

/// Whether `created_at` falls within the shared "just added" badge window
/// (`lisek_db::RECENT_BADGE_DAYS`) as of `now` — a movie's own "Nowość"
/// badge (Runda 9), the plain-data counterpart to a series' database-backed
/// `classify_series_badge`. A pure function taking `now` as a parameter
/// rather than calling `Utc::now()` internally, so it's unit-testable
/// without depending on the wall clock — same reasoning as
/// `lisek_db::classify_badge_from_flags`.
fn is_recently_added(created_at: DateTime<Utc>, now: DateTime<Utc>) -> bool {
    now - created_at <= Duration::days(lisek_db::RECENT_BADGE_DAYS)
}

async fn to_responses(
    pool: &lisek_db::PgPool,
    items: Vec<lisek_core::MediaItem>,
) -> Vec<MediaItemResponse> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        out.push(to_response(pool, item).await);
    }
    out
}

/// How many cards each row fetches — generous enough to fill and slightly
/// overflow a typical carousel viewport (`MediaRow`'s own Faza 4 skeleton
/// count defaulted to 10) without pulling a whole library's worth of items
/// into one response. Top 10 is, structurally, always exactly ten.
const ROW_LIMIT: i64 = 14;
const TOP10_LIMIT: i64 = 10;

#[derive(Serialize, utoipa::ToSchema)]
pub struct HomeFeedResponse {
    pub hero: Option<MediaItemResponse>,
    pub recently_added: Vec<MediaItemResponse>,
    pub top10: Vec<MediaItemResponse>,
    pub recommended: Vec<MediaItemResponse>,
    pub popular: Vec<MediaItemResponse>,
}

/// Everything the home page needs in one round trip — see `lisek-db`'s
/// `get_hero_item`/`list_recently_added`/`list_top_today`/
/// `list_recommended`/`list_popular` doc comments for exactly what each row
/// means and, for `top10`/`recommended`/`popular`, the honest placeholder
/// ranking each uses in the absence of any real watch-history/popularity
/// tracking (not yet built — see the roadmap).
#[utoipa::path(get,path="/api/home",tag="media",responses((status=200,body=HomeFeedResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn home_feed(
    _auth: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<HomeFeedResponse>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;

    let hero = lisek_db::get_hero_item(&pool)
        .await
        .map_err(|_| MediaError::Internal)?;
    let recently_added = lisek_db::list_recently_added(&pool, None, ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    let top10 = lisek_db::list_top_today(&pool, TOP10_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    let recommended = lisek_db::list_recommended(&pool, None, ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    let popular = lisek_db::list_popular(&pool, None, ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;

    let hero = match hero {
        Some(item) => Some(to_response(&pool, item).await),
        None => None,
    };

    Ok(Json(HomeFeedResponse {
        hero,
        recently_added: to_responses(&pool, recently_added).await,
        top10: to_responses(&pool, top10).await,
        recommended: to_responses(&pool, recommended).await,
        popular: to_responses(&pool, popular).await,
    }))
}

/// One library as the Navbar/`/library/[id]` grid consume it — deliberately
/// thinner than `admin::LibraryResponse`: `root_paths` are absolute
/// filesystem paths on the machine the server runs on, and have no business
/// reaching a non-admin browser tab (Runda 9). Every signed-in account gets
/// to see *that* a library exists and what the admin named it, never where
/// its files actually live.
#[derive(Serialize, utoipa::ToSchema)]
pub struct PublicLibraryResponse {
    pub id: Uuid,
    pub name: String,
    pub library_type: String,
}

impl From<lisek_core::Library> for PublicLibraryResponse {
    fn from(library: lisek_core::Library) -> Self {
        Self {
            id: library.id,
            name: library.name,
            library_type: library.library_type,
        }
    }
}

/// Every configured library, in the admin's shared display order — the
/// Navbar's data source (Runda 9), and how the `/library/[id]` grid's own
/// "which library is this" title/name gets resolved. Public, same as every
/// other route in this module; reads through the exact same
/// `lisek_db::list_libraries` (`ORDER BY sort_order, name`) that
/// `admin::list_libraries` already used for the Admin Panel — one query,
/// two response shapes, so the Navbar can never disagree with the admin's
/// own reorder buttons about which library comes first.
#[utoipa::path(get,path="/api/libraries",tag="media",responses((status=200,body=Vec<PublicLibraryResponse>),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn list_libraries(
    _auth: AuthUser,
    State(state): State<SharedState>,
) -> Result<Json<Vec<PublicLibraryResponse>>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;
    let libraries = lisek_db::list_libraries(&pool)
        .await
        .map_err(|_| MediaError::Internal)?;
    Ok(Json(libraries.into_iter().map(Into::into).collect()))
}

/// How many rows [`library_recent`]/[`library_recommended`]/
/// [`library_popular`] each fetch — reuses the exact same `ROW_LIMIT` the
/// home feed's own five curated rows use, so a library's rows are sized
/// consistently with the home page's (Runda 10).
const LIBRARY_ROW_LIMIT: i64 = ROW_LIMIT;

/// Default/maximum page size for [`library_items`]'s "all items" grid
/// (Runda 10). `DEFAULT` matches the "1–100 z …" counter shown in the
/// Runda 10 plan; `MAX` is defense-in-depth against an oversized
/// `page_size` query param — the shipped UI never sends one itself, there's
/// no page-size picker.
const DEFAULT_LIBRARY_PAGE_SIZE: i64 = 100;
const MAX_LIBRARY_PAGE_SIZE: i64 = 200;

/// Query parameters accepted by [`library_items`] — all optional, all
/// lenient on bad/missing input (see [`parse_library_sort`]/
/// [`parse_letter_filter`]): a malformed value here is a stale link or a
/// bug in this app's own UI, not something worth failing the whole request
/// over.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LibraryItemsQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub sort: Option<String>,
    pub letter: Option<String>,
}

/// Parses `?sort=`'s raw value into a [`lisek_db::LibrarySort`]. Unknown or
/// missing values default to `TitleAsc` — the same ordering
/// `list_items_by_library` (Runda 9) always used, so a plain, unparameterized
/// call to this endpoint sorts exactly as it always has.
fn parse_library_sort(raw: Option<&str>) -> lisek_db::LibrarySort {
    match raw {
        Some("title_desc") => lisek_db::LibrarySort::TitleDesc,
        Some("recent") => lisek_db::LibrarySort::RecentlyAdded,
        _ => lisek_db::LibrarySort::TitleAsc,
    }
}

/// Parses `?letter=`'s raw value into a single uppercase ASCII letter, or
/// `None` if it's missing, empty, longer than one character, or not a
/// letter at all (e.g. a stray `#`/digit) — `None` means "no filter", the
/// same as the param being absent.
fn parse_letter_filter(raw: Option<&str>) -> Option<char> {
    let value = raw?;
    let mut chars = value.chars();
    let first = chars.next()?;
    if chars.next().is_some() || !first.is_ascii_alphabetic() {
        return None;
    }
    Some(first.to_ascii_uppercase())
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct LibraryItemsResponse {
    pub library: PublicLibraryResponse,
    pub items: Vec<MediaItemResponse>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// One page of one library's browsable contents — the `/library/[id]`
/// page's "all items" grid data source. Runda 9 introduced this endpoint as
/// a flat, unpaginated, 500-item-capped fetch; Runda 10 adds real
/// pagination, sorting and an optional starts-with-letter filter (see
/// `lisek_db::list_items_by_library_page`/`count_items_by_library`), and
/// changes the response shape to carry `total`/`page`/`page_size` alongside
/// `items` so the frontend can render a "1–100 z 765"-style counter and
/// prev/next controls without a second request. This is a breaking change
/// to last round's response shape; `library_items` had no other known
/// caller besides this exact page, so changing it in place (rather than
/// versioning the route) is safe.
///
/// Bundles the library's own name/type alongside its items in one response
/// so the page can render its title without a second round trip. A library
/// id that doesn't exist (deleted, or simply wrong) is a real 404
/// ([`MediaError::LibraryNotFound`]) — unlike [`media_item_seasons`]'s
/// empty-array contract, there's a meaningful difference here between "this
/// library exists and has nothing in it yet" and "this library doesn't
/// exist", and the frontend needs to tell those apart to show the right
/// message.
#[utoipa::path(get,path="/api/libraries/{library_id}/items",tag="media",params(("library_id"=uuid::Uuid,Path),("page"=Option<i64>,Query),("sort"=Option<String>,Query),("letter"=Option<String>,Query)),responses((status=200,body=LibraryItemsResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn library_items(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<LibraryItemsQuery>,
) -> Result<Json<LibraryItemsResponse>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;

    let library = lisek_db::get_library(&pool, library_id)
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::LibraryNotFound)?;

    let sort = parse_library_sort(query.sort.as_deref());
    let letter = parse_letter_filter(query.letter.as_deref());
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query
        .page_size
        .unwrap_or(DEFAULT_LIBRARY_PAGE_SIZE)
        .clamp(1, MAX_LIBRARY_PAGE_SIZE);
    let offset = (page - 1) * page_size;

    let total = lisek_db::count_items_by_library(&pool, library_id, letter)
        .await
        .map_err(|_| MediaError::Internal)?;
    let items =
        lisek_db::list_items_by_library_page(&pool, library_id, sort, letter, page_size, offset)
            .await
            .map_err(|_| MediaError::Internal)?;

    Ok(Json(LibraryItemsResponse {
        library: library.into(),
        items: to_responses(&pool, items).await,
        total,
        page,
        page_size,
    }))
}

/// Response shape shared by [`library_recent`]/[`library_recommended`]/
/// [`library_popular`] (Runda 10) — deliberately identical to
/// [`LibraryItemsResponse`]'s `library`+`items` fields minus the pagination
/// ones, since these three are small curated rows, not a paginated grid.
/// Each of the three handlers independently looks up the library (rather
/// than sharing one earlier lookup) so a failure in one section's request
/// can never take another section down with it — see the Runda 10 plan's
/// reasoning for three separate endpoints instead of one bundled response.
#[derive(Serialize, utoipa::ToSchema)]
pub struct LibraryRowResponse {
    pub library: PublicLibraryResponse,
    pub items: Vec<MediaItemResponse>,
}

/// `/library/[id]`'s "Ostatnio dodane" row — `lisek_db::list_recently_added`
/// scoped to this library, reusing the exact same query (and therefore the
/// exact same `home_candidates` grouping and `created_at DESC` ordering) the
/// home feed's own "recently added" row already uses.
#[utoipa::path(get,path="/api/libraries/{library_id}/recent",tag="media",params(("library_id"=uuid::Uuid,Path)),responses((status=200,body=LibraryRowResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn library_recent(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<LibraryRowResponse>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;

    let library = lisek_db::get_library(&pool, library_id)
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::LibraryNotFound)?;

    let items = lisek_db::list_recently_added(&pool, Some(library_id), LIBRARY_ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;

    Ok(Json(LibraryRowResponse {
        library: library.into(),
        items: to_responses(&pool, items).await,
    }))
}

/// `/library/[id]`'s "Polecane dla Ciebie" row — `lisek_db::list_recommended`
/// scoped to this library, so a Filmy library never recommends a series and
/// vice versa. Same honest "no real taste model yet" caveat as the home
/// feed's own recommended row.
#[utoipa::path(get,path="/api/libraries/{library_id}/recommended",tag="media",params(("library_id"=uuid::Uuid,Path)),responses((status=200,body=LibraryRowResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn library_recommended(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<LibraryRowResponse>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;

    let library = lisek_db::get_library(&pool, library_id)
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::LibraryNotFound)?;

    let items = lisek_db::list_recommended(&pool, Some(library_id), LIBRARY_ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;

    Ok(Json(LibraryRowResponse {
        library: library.into(),
        items: to_responses(&pool, items).await,
    }))
}

/// `/library/[id]`'s "Popularne teraz" row — `lisek_db::list_popular` scoped
/// to this library. Same honest "no real popularity tracking yet" caveat as
/// the home feed's own popular row.
#[utoipa::path(get,path="/api/libraries/{library_id}/popular",tag="media",params(("library_id"=uuid::Uuid,Path)),responses((status=200,body=LibraryRowResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn library_popular(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(library_id): Path<Uuid>,
) -> Result<Json<LibraryRowResponse>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;

    let library = lisek_db::get_library(&pool, library_id)
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::LibraryNotFound)?;

    let items = lisek_db::list_popular(&pool, Some(library_id), LIBRARY_ROW_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;

    Ok(Json(LibraryRowResponse {
        library: library.into(),
        items: to_responses(&pool, items).await,
    }))
}

/// One item's full detail — the "Movie Detail View" the admin asked for
/// once cards became clickable. Public (any signed-in account, not just
/// admins — see this module's own doc comment), and deliberately reuses
/// [`MediaItemResponse`] rather than a bespoke type: the home feed already
/// carries everything this view shows (title/year/overview/season/episode
/// plus every image URL), so a separate response shape would just be the
/// same fields under different names. A dedicated route exists at all
/// because the home feed only ever returns a handful of rows — an item
/// reached by direct link (or simply not among the home feed's most-recent
/// picks) still needs to be fetchable on its own.
#[utoipa::path(get,path="/api/media/{item_id}",tag="media",params(("item_id"=uuid::Uuid,Path)),responses((status=200,body=MediaItemResponse),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn media_item_detail(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<MediaItemResponse>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;
    let item = lisek_db::get_media_item(&pool, item_id)
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::NotFound)?;
    Ok(Json(item.into()))
}

/// One episode, as the "Sezony" section renders it — see
/// `lisek_db::list_series_seasons`'s doc comment for exactly why this is
/// this thin (no title/thumbnail/runtime/overview: none of it exists
/// anywhere in this app yet). `poster_url`/`backdrop_url` reuse the exact
/// same `/api/media/{id}/...` routes as [`MediaItemResponse`] — `id` here
/// is a real `media_items` row, so they resolve the same way.
#[derive(Serialize, utoipa::ToSchema)]
pub struct SeriesEpisodeResponse {
    pub id: Uuid,
    pub episode: i32,
    pub end_episode: Option<i32>,
    pub poster_url: String,
    pub backdrop_url: String,
}

impl From<lisek_db::SeriesEpisode> for SeriesEpisodeResponse {
    fn from(episode: lisek_db::SeriesEpisode) -> Self {
        Self {
            poster_url: format!("/api/media/{}/poster", episode.id),
            backdrop_url: format!("/api/media/{}/backdrop", episode.id),
            id: episode.id,
            episode: episode.episode,
            end_episode: episode.end_episode,
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SeriesSeasonResponse {
    pub season: i32,
    pub episodes: Vec<SeriesEpisodeResponse>,
}

impl From<lisek_db::SeriesSeason> for SeriesSeasonResponse {
    fn from(season: lisek_db::SeriesSeason) -> Self {
        Self {
            season: season.season,
            episodes: season.episodes.into_iter().map(Into::into).collect(),
        }
    }
}

/// The "Sezony" section's own data — every season, and every season's
/// episodes, for the show `item_id` resolves to. Public, same as every
/// other route in this module; an empty array (not a 404) for an id that
/// doesn't exist, isn't a `media_items` row, or names a movie — see
/// `lisek_db::list_series_seasons`'s own doc comment on why that's the
/// right response rather than a distinct error case here.
#[utoipa::path(get,path="/api/media/{item_id}/seasons",tag="media",params(("item_id"=uuid::Uuid,Path)),responses((status=200,body=Vec<SeriesSeasonResponse>),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn media_item_seasons(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<Vec<SeriesSeasonResponse>>, MediaError> {
    let pool = state.pool().await.ok_or(MediaError::ServerNotReady)?;
    let seasons = lisek_db::list_series_seasons(&pool, item_id)
        .await
        .map_err(|_| MediaError::Internal)?;
    Ok(Json(seasons.into_iter().map(Into::into).collect()))
}

/// Serves one item's poster. `AuthUser` accepts a `?token=` query param
/// specifically so a plain `<img src>` can hit this route — see
/// `auth.rs::token_from_query`'s doc comment.
#[utoipa::path(get,path="/api/media/{item_id}/poster",tag="media",params(("item_id"=uuid::Uuid,Path)),responses((status=200,description="Image bytes"),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn poster_image(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Response {
    serve_item_image(
        &state,
        item_id,
        "poster.jpg",
        lisek_metadata::image_cache::poster_cache_path,
        "image/jpeg",
    )
    .await
}

/// Same as [`poster_image`] for the backdrop image.
#[utoipa::path(get,path="/api/media/{item_id}/backdrop",tag="media",params(("item_id"=uuid::Uuid,Path)),responses((status=200,description="Image bytes"),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn backdrop_image(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Response {
    serve_item_image(
        &state,
        item_id,
        "backdrop.jpg",
        lisek_metadata::image_cache::backdrop_cache_path,
        "image/jpeg",
    )
    .await
}

/// Same as [`poster_image`] for the transparent title-logo image — `.png`
/// media-folder filename and `image/png` content type, matching
/// `lisek_metadata::image_cache::logo_cache_path`'s own `.png` convention
/// (see that function's doc comment for why: real transparency, and every
/// logo this app downloads has already been filtered to PNG at the
/// provider layer). A 404 here just means this item has no logo — a very
/// common, unremarkable case (most titles don't have one cataloged, and
/// TVDB-only items never will yet — see `lisek_plugin_api::ProviderMatch`'s
/// own doc comment), which is exactly why the frontend treats it the same
/// as a missing poster: an `<img onerror>` fallback, not an error state.
#[utoipa::path(get,path="/api/media/{item_id}/logo",tag="media",params(("item_id"=uuid::Uuid,Path)),responses((status=200,description="Image bytes"),(status=400),(status=401),(status=403),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn logo_image(
    _auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Response {
    serve_item_image(
        &state,
        item_id,
        "logo.png",
        lisek_metadata::image_cache::logo_cache_path,
        "image/png",
    )
    .await
}

/// Tries the media-folder copy (`{media_folder_filename}` next to the
/// item's own `file_path`, looked up fresh via `lisek_db::
/// get_media_item_file_path`) before the local `config_dir` cache
/// (`cache_path_fn`). Any failure along the media-folder path — no pool
/// yet, no such item, no file path recorded, no file actually sitting
/// there — is treated identically as "no media-folder candidate, try the
/// cache" rather than as the request's final error; only the cache read at
/// the end decides between success and 404, exactly like before this
/// media-folder lookup was added. `content_type` is a fixed value per image
/// kind (`"image/jpeg"` for poster/backdrop, `"image/png"` for logo) rather
/// than sniffed from the bytes — cheap and correct, since every write path
/// already guarantees which format lands at each of these three cache
/// paths.
async fn serve_item_image(
    state: &SharedState,
    item_id: Uuid,
    media_folder_filename: &str,
    cache_path_fn: fn(&FsPath, Uuid) -> PathBuf,
    content_type: &'static str,
) -> Response {
    if let Some(bytes) = read_media_folder_image(state, item_id, media_folder_filename).await {
        return image_response(bytes, content_type);
    }

    let cache_path = cache_path_fn(&crate::state::config_dir(), item_id);
    match tokio::fs::read(&cache_path).await {
        Ok(bytes) => image_response(bytes, content_type),
        // Neither a media-folder copy nor a cached one — either this item
        // has no provider match yet, or no provider is configured at all.
        // A 404 is exactly right here: the frontend's `<img>` `onerror`
        // handler swaps in a placeholder, same as any other missing image.
        // Logged at `warn!` (not silently) with both paths this request
        // actually tried — a 404 on an item the admin knows was enriched
        // (`ScanSummaryResponse::images_saved` counted it) is exactly the
        // situation this line exists to make debuggable without adding
        // print statements: the reported path is byte-for-byte what
        // `lisek_metadata::image_cache::save_image` was given at scan
        // time, so a mismatch here (missing file, wrong drive, stale
        // `media_items.file_path`) is visible immediately instead of only
        // as an opaque 404 in the browser's network tab.
        Err(err) => {
            tracing::warn!(
                %item_id,
                cache_path = %cache_path.display(),
                %err,
                "no image at the media-folder location or the local cache — see this item's \
                 media_items.file_path and metadata_fallback if this is unexpected"
            );
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

async fn read_media_folder_image(
    state: &SharedState,
    item_id: Uuid,
    filename: &str,
) -> Option<Vec<u8>> {
    let pool = state.pool().await?;
    let file_path = match lisek_db::get_media_item_file_path(&pool, item_id).await {
        Ok(Some(file_path)) => file_path,
        Ok(None) => {
            tracing::debug!(%item_id, "no media_items row (or no file_path) for this id");
            return None;
        }
        Err(err) => {
            tracing::warn!(%item_id, %err, "failed to look up this item's file_path");
            return None;
        }
    };
    let Some(parent) = FsPath::new(&file_path).parent() else {
        tracing::warn!(%item_id, %file_path, "stored file_path has no parent directory");
        return None;
    };
    let media_path = parent.join(filename);
    match tokio::fs::read(&media_path).await {
        Ok(bytes) => Some(bytes),
        Err(err) => {
            tracing::debug!(
                %item_id,
                media_path = %media_path.display(),
                %err,
                "no media-folder image at this path — falling back to the local cache"
            );
            None
        }
    }
}

fn image_response(bytes: Vec<u8>, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            // Not content-addressed the way the old cache-only path was
            // (a media-folder file can be replaced by the admin at any
            // time outside a scan), but still only ever changes on a
            // fresh scan+enrichment cycle in the normal case — an hour of
            // browser caching remains a reasonable trade.
            (header::CACHE_CONTROL, "private, max-age=3600"),
        ],
        bytes,
    )
        .into_response()
}

#[derive(Debug)]
pub enum MediaError {
    ServerNotReady,
    /// No `media_items` row with this id — [`media_item_detail`]'s own
    /// "the item this link points to doesn't exist" case, distinct from
    /// [`MediaError::Internal`] so the frontend can show a real "not
    /// found" state instead of a generic error banner.
    NotFound,
    /// No `libraries` row with this id — [`library_items`]'s own "not
    /// found" case (Runda 9), kept distinct from [`MediaError::NotFound`]
    /// so the `/library/[id]` page can tell "this library doesn't exist"
    /// apart from "this media item doesn't exist" if it ever needs to.
    LibraryNotFound,
    Internal,
}

impl IntoResponse for MediaError {
    fn into_response(self) -> Response {
        let (status, error_code, message) = match self {
            MediaError::ServerNotReady => (
                StatusCode::SERVICE_UNAVAILABLE,
                "server_not_ready",
                "The server isn't fully configured yet.",
            ),
            MediaError::NotFound => (
                StatusCode::NOT_FOUND,
                "not_found",
                "No such media item exists.",
            ),
            MediaError::LibraryNotFound => (
                StatusCode::NOT_FOUND,
                "library_not_found",
                "No such library is configured.",
            ),
            MediaError::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Something went wrong while loading the home feed.",
            ),
        };

        (
            status,
            Json(json!({ "error_code": error_code, "message": message })),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_movie_added_this_instant_is_recent() {
        let now = Utc::now();
        assert!(is_recently_added(now, now));
    }

    #[test]
    fn a_movie_added_exactly_at_the_window_boundary_is_still_recent() {
        let now = Utc::now();
        let created_at = now - Duration::days(lisek_db::RECENT_BADGE_DAYS);
        assert!(is_recently_added(created_at, now));
    }

    #[test]
    fn a_movie_added_one_second_past_the_window_is_no_longer_recent() {
        let now = Utc::now();
        let created_at = now - Duration::days(lisek_db::RECENT_BADGE_DAYS) - Duration::seconds(1);
        assert!(!is_recently_added(created_at, now));
    }

    #[test]
    fn a_movie_added_long_ago_is_not_recent() {
        let now = Utc::now();
        let created_at = now - Duration::days(365);
        assert!(!is_recently_added(created_at, now));
    }

    #[test]
    fn library_sort_defaults_to_title_asc_for_missing_or_unknown_values() {
        assert_eq!(parse_library_sort(None), lisek_db::LibrarySort::TitleAsc);
        assert_eq!(
            parse_library_sort(Some("nonsense")),
            lisek_db::LibrarySort::TitleAsc
        );
        assert_eq!(
            parse_library_sort(Some("")),
            lisek_db::LibrarySort::TitleAsc
        );
    }

    #[test]
    fn library_sort_recognizes_title_desc_and_recent() {
        assert_eq!(
            parse_library_sort(Some("title_desc")),
            lisek_db::LibrarySort::TitleDesc
        );
        assert_eq!(
            parse_library_sort(Some("recent")),
            lisek_db::LibrarySort::RecentlyAdded
        );
    }

    #[test]
    fn letter_filter_uppercases_a_single_letter() {
        assert_eq!(parse_letter_filter(Some("a")), Some('A'));
        assert_eq!(parse_letter_filter(Some("Z")), Some('Z'));
    }

    #[test]
    fn letter_filter_rejects_missing_empty_multi_char_or_non_letter_values() {
        assert_eq!(parse_letter_filter(None), None);
        assert_eq!(parse_letter_filter(Some("")), None);
        assert_eq!(parse_letter_filter(Some("ab")), None);
        assert_eq!(parse_letter_filter(Some("1")), None);
        assert_eq!(parse_letter_filter(Some("#")), None);
    }
}
