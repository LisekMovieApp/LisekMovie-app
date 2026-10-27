//! "Fix match" (Identify): an administrator searches the metadata
//! providers by hand and pins the right entry on a film or a whole series.
//!
//! - `GET /api/providers/search`: candidates straight from the provider
//!   plugins (`MetadataProvider::search`). A query that is an id — an IMDb
//!   id (`tt0425622`), a prefixed id (`tvdb:60620`), a provider page link,
//!   or a plain number with one provider chosen — goes to
//!   `MetadataProvider::lookup_by_id` instead (`lisek_plugin_api::
//!   parse_id_query`).
//! - `POST /api/media/{id}/identify`: looks the picked id up
//!   (`MetadataProvider::lookup`), renames the item(s) to the provider's
//!   title, throws the old artwork away and saves the new match
//!   (`metadata_enrichment::apply_match`: fields, images, collection via
//!   `sync_artwork`). The items are locked and pinned to that provider, so
//!   neither a rescan nor the six-hourly refresh searches by the file name
//!   again. A series is fixed as a whole: every episode of it.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lisek_db::EnrichTarget;
use lisek_metadata::image_cache::{backdrop_cache_path, logo_cache_path, poster_cache_path};
use lisek_plugin_api::{IdQuery, MediaKind, ProviderMatch, parse_id_query};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AdminUser;
use crate::metadata_enrichment::{self, EnrichCache};
use crate::playback::{PlaybackError, pool};
use crate::provider_registry;
use crate::state::{SharedState, config_dir};

const MAX_QUERY_CHARS: usize = 200;

fn error(status: StatusCode, code: &'static str) -> PlaybackError {
    PlaybackError(status, code)
}

fn parse_kind(raw: &str) -> Option<MediaKind> {
    match raw {
        "movie" => Some(MediaKind::Movie),
        "series" => Some(MediaKind::Series),
        _ => None,
    }
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ProviderSearchQuery {
    /// Title to search for, as typed — or an id: `tt0425622` (IMDb),
    /// `tvdb:60620`, `anilist:21`, a provider page link, or a plain number
    /// (the chosen provider's own id; also searched as a title).
    pub query: String,
    /// Release (or first-air) year; narrows the search.
    pub year: Option<i32>,
    /// `movie` or `series`.
    pub kind: String,
    /// Only this provider (`tmdb`, `tvdb`, `imdb`, `anilist`...); every
    /// configured one when absent.
    pub provider: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderCandidateResponse {
    pub provider: String,
    pub provider_id: String,
    pub title: String,
    pub year: Option<i32>,
    pub overview: String,
    pub poster_url: Option<String>,
    /// `id`: the provider resolved the id typed as the query; `search`: a
    /// title search result.
    pub matched_by: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderSearchFailure {
    pub provider: String,
    pub error: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderSearchResponse {
    /// Candidates of every provider asked: id matches first, then each
    /// provider's search results, best first.
    pub results: Vec<ProviderCandidateResponse>,
    /// The query was read as an id (`imdb` or `numeric`), not only as a
    /// title.
    pub id_kind: Option<String>,
    /// Providers whose search failed (network, rejected key).
    pub failures: Vec<ProviderSearchFailure>,
}

fn id_kind(id: &lisek_plugin_api::DirectId) -> &'static str {
    match id {
        lisek_plugin_api::DirectId::Imdb(_) => "imdb",
        lisek_plugin_api::DirectId::Numeric(_) => "numeric",
    }
}

/// What one Fix match query asks of the providers.
#[derive(Debug, PartialEq)]
struct SearchPlan<'a> {
    /// Look this id up (`lookup_by_id`).
    id: Option<lisek_plugin_api::DirectId>,
    /// Also (or only) search the query as a title.
    search_text: bool,
    /// Only this provider.
    provider: Option<&'a str>,
}

impl<'a> SearchPlan<'a> {
    /// - an IMDb id, a prefixed id or a link: lookup only, at the provider
    ///   the prefix or link names (else every chosen one);
    /// - a bare number: a title too ("1917", "300"), so the text is always
    ///   searched; it is looked up as an id only when one provider is
    ///   chosen — a number means something different at every provider;
    /// - anything else: a title search.
    fn new(id: Option<IdQuery>, chosen: Option<&'a str>) -> Self {
        match id {
            Some(IdQuery { ambiguous: true, id, .. }) => Self {
                id: chosen.map(|_| id),
                search_text: true,
                provider: chosen,
            },
            Some(IdQuery { id, provider, .. }) => Self {
                id: Some(id),
                search_text: false,
                provider: chosen.or(provider),
            },
            None => Self {
                id: None,
                search_text: true,
                provider: chosen,
            },
        }
    }
}

/// Search the configured metadata providers by hand ("Fix match").
#[utoipa::path(get,path="/api/providers/search",tag="admin",params(ProviderSearchQuery),responses((status=200,body=ProviderSearchResponse),(status=400),(status=401),(status=403),(status=503)),security(("lisek_auth"=[])))]
pub async fn search(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Query(query): Query<ProviderSearchQuery>,
) -> Result<Json<ProviderSearchResponse>, PlaybackError> {
    let title = query.query.trim();
    let kind = parse_kind(&query.kind).ok_or(error(StatusCode::BAD_REQUEST, "invalid_kind"))?;
    if title.is_empty() || title.chars().count() > MAX_QUERY_CHARS {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_query"));
    }
    let pool = pool(&state).await?;
    let providers = provider_registry::enabled_provider_keys(&pool, &state.secrets).await?;
    let plan = SearchPlan::new(parse_id_query(title), query.provider.as_deref());
    let providers: Vec<_> = providers
        .into_iter()
        .filter(|(key, _)| plan.provider.is_none_or(|wanted| wanted == key))
        .collect();
    if providers.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "provider_not_configured"));
    }

    let mut response = ProviderSearchResponse {
        results: Vec::new(),
        id_kind: plan.id.as_ref().map(|id| id_kind(id).to_string()),
        failures: Vec::new(),
    };
    let language = auth.user.ui_language.clone();
    let mut seen = std::collections::HashSet::new();
    for (key, api_key) in providers {
        let Some(provider) = provider_registry::build_provider(&key, &api_key) else {
            continue;
        };
        let mut push = |candidate: lisek_plugin_api::ProviderCandidate, matched_by: &str| {
            if seen.insert((key.clone(), candidate.provider_id.clone())) {
                response.results.push(ProviderCandidateResponse {
                    provider: key.clone(),
                    provider_id: candidate.provider_id,
                    title: candidate.title,
                    year: candidate.year,
                    overview: candidate.overview,
                    poster_url: candidate.poster_url,
                    matched_by: matched_by.to_string(),
                });
            }
        };
        let mut failure = None;
        if let Some(id) = &plan.id {
            tracing::debug!(provider = key, ?id, ?kind, "fix match: looking up an id");
            match provider.lookup_by_id(kind, id, &language).await {
                Ok(Some(found)) => push(found, "id"),
                Ok(None) => {}
                Err(err) => failure = Some(err),
            }
        }
        if plan.search_text && failure.is_none() {
            tracing::debug!(provider = key, title, year = query.year, ?kind, "fix match: searching");
            match provider.search(kind, title, query.year, &language).await {
                Ok(found) => found.into_iter().for_each(|c| push(c, "search")),
                Err(err) => failure = Some(err),
            }
        }
        if let Some(err) = failure {
            tracing::warn!(provider = key, %err, "fix match: provider search failed");
            response.failures.push(ProviderSearchFailure {
                provider: key,
                error: err.to_string(),
            });
        }
    }
    // Id matches first, whichever provider found them.
    response.results.sort_by_key(|c| c.matched_by != "id");
    Ok(Json(response))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct IdentifyRequest {
    /// `tmdb` or `tvdb`.
    pub provider: String,
    /// The provider's id of the right film or series (from the search).
    pub provider_id: String,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct IdentifyResponse {
    pub item_id: Uuid,
    pub title: String,
    pub year: Option<i32>,
    /// Films or episodes updated (a series counts every episode).
    pub items_updated: usize,
    /// At least one new image was saved.
    pub image_saved: bool,
    /// Parts that did not work out (`poster_download_failed`,
    /// `backdrop_save_failed`...): the match is saved, these are not.
    pub warnings: Vec<String>,
}

/// The provider's own title and year from a lookup (`fields.title`,
/// `fields.year`).
fn title_and_year(matched: &ProviderMatch) -> Option<(String, Option<i32>)> {
    let title = matched
        .fields
        .get("title")?
        .as_str()?
        .trim()
        .to_string();
    if title.is_empty() {
        return None;
    }
    let year = matched
        .fields
        .get("year")
        .and_then(|y| y.as_i64())
        .and_then(|y| i32::try_from(y).ok());
    Some((title, year))
}

/// Pin the right provider entry on a film or a whole series ("Fix match").
#[utoipa::path(post,path="/api/media/{item_id}/identify",tag="admin",params(("item_id"=Uuid,Path)),request_body=IdentifyRequest,responses((status=200,body=IdentifyResponse),(status=400),(status=401),(status=403),(status=404),(status=502),(status=503)),security(("lisek_auth"=[])))]
pub async fn identify(
    AdminUser(auth): AdminUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
    Json(req): Json<IdentifyRequest>,
) -> Result<Json<IdentifyResponse>, PlaybackError> {
    let provider_id = req.provider_id.trim();
    if provider_id.is_empty() || provider_id.len() > 64 {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_provider_id"));
    }
    let pool = pool(&state).await?;
    let item = lisek_db::get_media_item_detail(&pool, item_id)
        .await?
        .ok_or(error(StatusCode::NOT_FOUND, "media_not_found"))?;
    let kind = if item.item_type == "movie" {
        MediaKind::Movie
    } else {
        MediaKind::Series
    };
    let api_key = provider_registry::enabled_provider_keys(&pool, &state.secrets)
        .await?
        .into_iter()
        .find(|(key, _)| *key == req.provider)
        .map(|(_, api_key)| api_key)
        .ok_or(error(StatusCode::BAD_REQUEST, "provider_not_configured"))?;
    let provider = provider_registry::build_provider(&req.provider, &api_key)
        .ok_or(error(StatusCode::BAD_REQUEST, "provider_not_configured"))?;

    let matched = match provider
        .lookup(kind, provider_id, &auth.user.ui_language)
        .await
    {
        Ok(Some(matched)) => matched,
        Ok(None) => return Err(error(StatusCode::NOT_FOUND, "provider_item_not_found")),
        Err(err) => {
            tracing::warn!(provider = req.provider, provider_id, %err, "fix match: lookup failed");
            return Err(error(StatusCode::BAD_GATEWAY, "provider_failed"));
        }
    };
    let (title, year) = title_and_year(&matched).unwrap_or((item.title.clone(), item.year));

    let items = match kind {
        MediaKind::Movie => vec![item_id],
        MediaKind::Series => {
            let episodes = lisek_db::episodes::series_episode_ids(&pool, item_id).await?;
            if episodes.is_empty() { vec![item_id] } else { episodes }
        }
    };
    tracing::info!(
        %item_id,
        provider = req.provider,
        provider_id,
        old_title = item.title,
        new_title = title,
        items = items.len(),
        admin = auth.user.username,
        "fix match: applying"
    );
    lisek_db::apply_identification(&pool, &items, &title, year, &req.provider).await?;

    let library = lisek_db::get_library(&pool, item.library_id)
        .await?
        .ok_or(error(StatusCode::NOT_FOUND, "library_not_found"))?;
    let target = match kind {
        MediaKind::Movie => EnrichTarget::Movie {
            title: title.clone(),
            year,
        },
        MediaKind::Series => EnrichTarget::Series {
            title: title.clone(),
        },
    };
    let config = config_dir();
    let http = reqwest::Client::new();
    let mut cache = EnrichCache::default();
    let mut image_saved = false;
    let mut warnings: Vec<String> = Vec::new();
    let mut unsaved = 0usize;
    for id in &items {
        // The old match's artwork goes: a new match without a logo must not
        // keep showing the wrong film's one.
        for stale in [
            poster_cache_path(&config, *id),
            backdrop_cache_path(&config, *id),
            logo_cache_path(&config, *id),
        ] {
            let _ = tokio::fs::remove_file(stale).await;
        }
        let Some(file_path) = lisek_db::get_media_item_file_path(&pool, *id).await? else {
            continue;
        };
        let file_path = std::path::PathBuf::from(file_path);
        let artwork_dir = match kind {
            MediaKind::Movie => file_path.parent().map(std::path::Path::to_path_buf),
            MediaKind::Series => lisek_metadata::layout::series_folder(&file_path),
        };
        if library.save_metadata_in_media_folder
            && matched.logo_url.is_none()
            && let Some(dir) = artwork_dir
        {
            let _ = tokio::fs::remove_file(dir.join("logo.png")).await;
        }
        let outcome = metadata_enrichment::apply_match(
            &mut cache,
            &pool,
            &http,
            &config,
            *id,
            &target,
            &file_path,
            library.save_metadata_in_media_folder,
            &req.provider,
            &api_key,
            &auth.user.ui_language,
            &matched,
        )
        .await;
        image_saved |= outcome.image_saved;
        if !outcome.metadata_saved {
            unsaved += 1;
        }
        for problem in outcome.problems {
            if !warnings.contains(&problem) {
                warnings.push(problem);
            }
        }
    }
    metadata_enrichment::prune_collections(&pool, &config).await;

    if unsaved > 0 {
        tracing::error!(%item_id, provider = req.provider, unsaved, "fix match: provider data not saved");
        return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "identify_save_failed"));
    }
    if matched.poster_url.is_some() && !image_saved && !warnings.iter().any(|w| w.starts_with("poster")) {
        warnings.push("artwork_not_saved".to_string());
    }
    if !warnings.is_empty() {
        tracing::warn!(%item_id, provider = req.provider, ?warnings, "fix match applied with problems");
    } else {
        tracing::info!(%item_id, provider = req.provider, title, "fix match applied");
    }

    Ok(Json(IdentifyResponse {
        item_id,
        title,
        year,
        items_updated: items.len(),
        image_saved,
        warnings,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_go_to_lookups_and_titles_to_searches() {
        use lisek_plugin_api::DirectId;
        let plan = |q: &str, chosen: Option<&'static str>| SearchPlan::new(parse_id_query(q), chosen);

        let imdb = plan("tt0425622", None);
        assert_eq!(imdb.id, Some(DirectId::Imdb("tt0425622".into())));
        assert!(!imdb.search_text);
        assert_eq!(imdb.provider, None);

        let prefixed = plan("tvdb:60620", None);
        assert_eq!((prefixed.id, prefixed.search_text, prefixed.provider), (Some(DirectId::Numeric("60620".into())), false, Some("tvdb")));
        // The provider picked in the form wins over the prefix.
        assert_eq!(plan("tvdb:60620", Some("tmdb")).provider, Some("tmdb"));

        let number_alone = plan("1917", None);
        assert_eq!((number_alone.id, number_alone.search_text), (None, true));
        let number_at_tvdb = plan("60620", Some("tvdb"));
        assert_eq!((number_at_tvdb.id, number_at_tvdb.search_text), (Some(DirectId::Numeric("60620".into())), true));

        let title = plan("Star Wars", Some("tmdb"));
        assert_eq!((title.id, title.search_text, title.provider), (None, true, Some("tmdb")));
    }

    #[test]
    fn reads_the_providers_title_and_year() {
        let matched = ProviderMatch {
            fields: serde_json::json!({"title": " Gwiezdne wojny ", "year": 1977}),
            ..ProviderMatch::default()
        };
        assert_eq!(title_and_year(&matched), Some(("Gwiezdne wojny".to_string(), Some(1977))));
        let untitled = ProviderMatch {
            fields: serde_json::json!({"title": "", "year": 1977}),
            ..ProviderMatch::default()
        };
        assert_eq!(title_and_year(&untitled), None);
        assert_eq!(parse_kind("series"), Some(MediaKind::Series));
        assert_eq!(parse_kind("episode"), None);
    }
}
