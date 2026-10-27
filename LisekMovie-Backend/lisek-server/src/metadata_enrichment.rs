//! Fills in a freshly-scanned `media_items` row with real metadata: tries
//! every enabled, configured provider in the admin's priority order (see
//! `lisek_db::list_enabled_provider_keys`) until one returns a match, merges
//! its fields into the row, then saves its poster/backdrop/logo images —
//! unless the item has its own `forced_provider` set (the metadata editor's
//! per-item override) or individual `locked_fields`, in which case
//! [`enrich_item_cached`] narrows to just that provider or protects just that
//! field respectively; see that function's own doc comment for both.
//!
//! As of Faza 5 round 2, "saves its images" means writing directly next to
//! the video file by default: `poster.jpg`/`backdrop.jpg`/`logo.png` (see
//! [`media_folder_destination`]), with the app's own local `config_dir`
//! cache used either as the library's deliberate choice (`save_in_media_folder
//! == false`) or as an automatic fallback when the media-folder write itself
//! is denied (see `lisek_metadata::image_cache::save_image`) — the latter
//! case gets persisted onto the item's `metadata_fallback` flag so a future
//! Admin Panel warning view can list it. Which image a provider hands back
//! (language-prioritized or not) is entirely that provider extension's own
//! concern as of Faza 7 — this module just downloads and writes whatever
//! `poster_url`/`backdrop_url`/`logo_url` it's given, the same way for
//! every provider. `banner_url` is deliberately never downloaded here: no
//! provider extension currently populates it — see
//! `lisek_plugin_api::ProviderMatch`'s own doc comment for why.
//!
//! Called once per item from the scan worker (`scan_jobs`) and the
//! background refresh, through an [`EnrichCache`]: every episode of a show
//! shares one provider lookup and one image download per URL. A provider failing (network error,
//! rejected key, no match) for one item is logged and treated as "try the
//! next provider" / "leave this item without provider metadata for now" —
//! it never aborts the rest of the scan, since one bad provider or one
//! unmatched title shouldn't cost the admin every other item's data.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lisek_db::{EnrichTarget, PgPool};
use lisek_metadata::image_cache::{
    backdrop_cache_path, download_image, fetch_image, logo_cache_path, poster_cache_path,
    store_image,
};
use lisek_metadata::layout;
use lisek_plugin_api::{MovieQuery, ProviderEpisode, ProviderMatch, ProviderPerson, SeriesQuery};
use uuid::Uuid;

use crate::provider_registry;

/// Builds `provider_key`'s extension (see [`provider_registry`]) and asks
/// it for `target`, dispatching to `find_movie`/`find_series` depending on
/// which variant `target` is. `None` covers an unregistered `provider_key`,
/// no match, and a failed lookup alike — all three are "try the next
/// provider in the admin's priority list" as far as [`enrich_item_cached`] is
/// concerned; which one actually happened is only ever visible in the
/// `tracing::warn!` line this function emits.
async fn try_provider(
    provider_key: &str,
    api_key: &str,
    target: &EnrichTarget,
    primary_language: &str,
) -> Option<ProviderMatch> {
    let Some(provider) = provider_registry::build_provider(provider_key, api_key) else {
        tracing::warn!(
            provider = provider_key,
            "no metadata client for this provider key"
        );
        return None;
    };

    let result = match target {
        EnrichTarget::Movie { title, year } => {
            provider
                .find_movie(
                    &MovieQuery {
                        title: title.as_str(),
                        year: *year,
                    },
                    primary_language,
                )
                .await
        }
        EnrichTarget::Series { title } => {
            provider
                .find_series(
                    &SeriesQuery {
                        title: title.as_str(),
                    },
                    primary_language,
                )
                .await
        }
    };

    match result {
        Ok(Some(matched)) => Some(matched),
        // A clean, successful lookup that simply found nothing — never
        // logged before this, which made a run like "9/10 items matched"
        // impossible to debug from the logs alone: nothing recorded which
        // title actually went out over the wire for the one that didn't
        // match, so the only way to find out was to re-derive it by hand
        // from the filename and the parser's own rules. Logged with the
        // *exact* query this provider was asked (post-parser, post any
        // year the parser extracted) — a mismatch between this and the
        // provider's own listing for the title is the single fastest way
        // to tell "the parser mangled the title" apart from "this title
        // genuinely isn't in this provider's catalog."
        Ok(None) => {
            match target {
                EnrichTarget::Movie { title, year } => tracing::warn!(
                    provider = provider_key,
                    title,
                    ?year,
                    "provider returned no match for this title/year"
                ),
                EnrichTarget::Series { title } => tracing::warn!(
                    provider = provider_key,
                    title,
                    "provider returned no match for this title"
                ),
            }
            None
        }
        Err(err) => {
            tracing::warn!(provider = provider_key, %err, "metadata lookup failed");
            None
        }
    }
}

/// Where a provider match's poster/backdrop should be written inside the
/// media folder itself — `None` when the library has media-folder saving
/// turned off, or when `file_path` unexpectedly has no parent directory
/// (never true for a real scanned file, but this stays a clean `None`
/// rather than a panic either way). `filename` is the caller-chosen
/// media-folder filename (`"poster.jpg"`, `"backdrop.jpg"`).
fn media_folder_destination(
    file_path: &Path,
    filename: &str,
    save_in_media_folder: bool,
) -> Option<PathBuf> {
    if !save_in_media_folder {
        return None;
    }
    file_path.parent().map(|dir| dir.join(filename))
}

/// A series' artwork (`poster.jpg`, `backdrop.jpg`, `logo.png`, season
/// posters) goes into the series folder, never a season folder.
fn series_folder_destination(
    episode_file: &Path,
    filename: &str,
    save_in_media_folder: bool,
) -> Option<PathBuf> {
    if !save_in_media_folder {
        return None;
    }
    layout::series_folder(episode_file).map(|dir| dir.join(filename))
}

/// The provider's facts about one episode (for a double episode, the first
/// one), as `provider_data.episode`: title, overview, runtime_minutes,
/// rating, still_url, air_date. `None` when the provider has none or the
/// season lookup failed (the episode then keeps what it had).
async fn episode_facts(
    cache: &mut EnrichCache,
    provider_key: &str,
    api_key: &str,
    language: &str,
    series_fields: &serde_json::Value,
    place: &lisek_db::seasons::Placement,
) -> Option<serde_json::Value> {
    let series_id = match series_fields.get(format!("{provider_key}_id"))? {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) if !s.is_empty() => s.clone(),
        _ => return None,
    };
    let key = (provider_key.to_owned(), series_id.clone(), place.season);
    if !cache.season_episodes.contains_key(&key) {
        let found = match provider_registry::build_provider(provider_key, api_key) {
            Some(provider) => match provider.season_episodes(&series_id, place.season, language).await {
                Ok(episodes) => Some(episodes),
                Err(err) => {
                    tracing::warn!(provider = provider_key, series_id, season = place.season, %err, "season episodes lookup failed");
                    None
                }
            },
            None => None,
        };
        cache.season_episodes.insert(key.clone(), found);
    }
    let episodes = cache.season_episodes.get(&key)?.as_ref()?;
    let episode = episodes.iter().find(|e| e.episode == place.episode)?;
    let title = match place.end_episode {
        Some(end) if end > place.episode => {
            let titles: Vec<&str> = (place.episode..=end)
                .filter_map(|n| episodes.iter().find(|e| e.episode == n)?.title.as_deref())
                .collect();
            (!titles.is_empty()).then(|| titles.join(" / "))
        }
        _ => episode.title.clone(),
    };
    Some(serde_json::json!({
        "title": title,
        "overview": episode.overview,
        "runtime_minutes": episode.runtime_minutes,
        "rating": episode.rating,
        "still_url": episode.still_url,
        "air_date": episode.air_date,
    }))
}

/// The episode's still, in the local cache only.
async fn sync_episode_still(
    http: &reqwest::Client,
    config_dir: &Path,
    item_id: Uuid,
    previous: Option<String>,
    wanted: Option<&str>,
) {
    let path = layout::episode_still_path(config_dir, &item_id.to_string());
    let label = item_id.to_string();
    let _ = sync_artwork(http, &path, &label, previous, wanted).await;
}

/// Once per series and pass: season names and posters from the provider
/// (`<series folder>/season01-poster.jpg`, or the local cache) and removal
/// of the copies of the series' artwork older versions wrote into season
/// folders.
#[allow(clippy::too_many_arguments)]
async fn sync_series_layout(
    cache: &mut EnrichCache,
    pool: &PgPool,
    http: &reqwest::Client,
    config_dir: &Path,
    series_id: Uuid,
    target: &EnrichTarget,
    file_path: &Path,
    save_in_media_folder: bool,
    matched: &ProviderMatch,
) {
    let (stored, files) = match lisek_db::seasons::series_layout(pool, series_id).await {
        Ok(layout) => layout,
        Err(err) => {
            tracing::warn!(%series_id, %err, "cannot read the seasons of a series");
            return;
        }
    };
    let series_dir = layout::series_folder(file_path);
    let series_key = series_id.to_string();
    for season in &stored {
        let provided = matched.seasons.iter().find(|s| s.number == season.number);
        if let Some(p) = provided
            && let Err(err) = lisek_db::seasons::set_season_metadata(
                pool,
                series_id,
                season.number,
                p.name.as_deref(),
                p.overview.as_deref(),
                p.air_year,
            )
            .await
        {
            tracing::warn!(%series_id, season = season.number, %err, "cannot save season metadata");
        }
        // No season data at all (another provider, a failed lookup): keep
        // whatever poster is there.
        if matched.seasons.is_empty() {
            continue;
        }
        let wanted = provided.and_then(|p| p.poster_url.as_deref());
        let name = layout::season_poster_name(season.number);
        let media_dest = save_in_media_folder
            .then(|| series_dir.as_ref().map(|d| d.join(&name)))
            .flatten();
        let cache_dest = layout::season_poster_cache_path(config_dir, &series_key, season.number);
        let Some(url) = wanted else {
            if season.poster_url.is_some() {
                let _ = tokio::fs::remove_file(&cache_dest).await;
                if let Some(dest) = &media_dest {
                    let _ = tokio::fs::remove_file(dest).await;
                }
                let _ = lisek_db::seasons::set_season_poster_url(pool, series_id, season.number, None).await;
            }
            continue;
        };
        let on_disk = match &media_dest {
            Some(dest) => tokio::fs::try_exists(dest).await.unwrap_or(false),
            None => false,
        } || tokio::fs::try_exists(&cache_dest).await.unwrap_or(false);
        if season.poster_url.as_deref() == Some(url) && on_disk {
            continue;
        }
        let bytes = match cache.image(http, target, url).await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(%series_id, season = season.number, %err, "cannot download a season poster");
                continue;
            }
        };
        match store_image(&bytes, media_dest.as_deref(), &cache_dest).await {
            Ok(_) => {
                tracing::debug!(%series_id, season = season.number, file = name, "season poster saved");
                let _ = lisek_db::seasons::set_season_poster_url(pool, series_id, season.number, Some(url)).await;
            }
            Err(err) => tracing::warn!(%series_id, season = season.number, %err, "cannot save a season poster"),
        }
    }
    if save_in_media_folder && let Some(series_dir) = series_dir {
        remove_legacy_season_artwork(&series_dir, &files).await;
    }
}

/// Before Faza 10 the series' `poster.jpg`/`backdrop.jpg`/`logo.png` were
/// written next to the episodes, i.e. into every season folder. A copy
/// identical to the one now in the series folder is removed; anything
/// else (an administrator's own season poster) stays.
async fn remove_legacy_season_artwork(series_dir: &Path, episode_files: &[String]) {
    let mut folders: Vec<PathBuf> = episode_files
        .iter()
        .filter_map(|f| Path::new(f).parent().map(Path::to_path_buf))
        .filter(|dir| dir != series_dir)
        .collect();
    folders.sort();
    folders.dedup();
    for folder in folders {
        for name in layout::LEGACY_SEASON_FOLDER_FILES {
            let old = folder.join(name);
            let Ok(old_bytes) = tokio::fs::read(&old).await else { continue };
            let Ok(current) = tokio::fs::read(series_dir.join(name)).await else { continue };
            if old_bytes == current {
                match tokio::fs::remove_file(&old).await {
                    Ok(()) => tracing::info!(file = %old.display(), "removed a copy of the series artwork from a season folder"),
                    Err(err) => tracing::warn!(file = %old.display(), %err, "cannot remove old artwork from a season folder"),
                }
            }
        }
    }
}

/// Memo for one enrichment pass. A scan of a 20-episode season used to
/// search the provider 20 times and download the same three images 20
/// times; with this, once.
///
/// Only series lookups are memoized (a movie target is unique per item).
/// Image bytes are kept only for the series currently being processed —
/// callers pass targets grouped by series (`persist_scan_results` and
/// `list_unlocked_items_for_refresh` both sort that way) — so memory stays
/// bounded by one show's artwork.
/// `(target, forced_provider)` — a forced provider changes the answer.
type LookupKey = (EnrichTarget, Option<String>);
/// `(provider_key, match)`.
type Found = (String, ProviderMatch);

#[derive(Default)]
pub struct EnrichCache {
    lookups: HashMap<LookupKey, Option<Found>>,
    images_for: Option<EnrichTarget>,
    images: HashMap<String, Arc<Vec<u8>>>,
    /// Media-folder files written this pass. Episodes in one season folder
    /// share `poster.jpg`; it is written once.
    written: HashSet<PathBuf>,
    /// Collections whose artwork was checked this pass (every film of a
    /// collection names the same images).
    collections_done: HashSet<Uuid>,
    /// People whose profile photo was checked this pass (an actor of a
    /// series is credited on every episode).
    people_done: HashSet<Uuid>,
    /// Series whose season data, season posters and legacy artwork were
    /// handled this pass.
    series_done: HashSet<Uuid>,
    /// A season's episodes from the provider, once per pass:
    /// `(provider, series id, season)` → episodes (`None`: lookup failed).
    season_episodes: HashMap<(String, String, i32), Option<Vec<ProviderEpisode>>>,
}

impl EnrichCache {
    async fn image(
        &mut self,
        http: &reqwest::Client,
        target: &EnrichTarget,
        url: &str,
    ) -> Result<Arc<Vec<u8>>, lisek_metadata::image_cache::ImageCacheError> {
        let cacheable = matches!(target, EnrichTarget::Series { .. });
        if cacheable && self.images_for.as_ref() != Some(target) {
            self.images.clear();
            self.images_for = Some(target.clone());
        }
        if let Some(bytes) = self.images.get(url) {
            return Ok(bytes.clone());
        }
        let bytes = Arc::new(fetch_image(http, url).await?);
        if cacheable {
            self.images.insert(url.to_owned(), bytes.clone());
        }
        Ok(bytes)
    }
}

/// Tries `providers` (an admin- or library-priority-ordered
/// `(provider_key, api_key)` list — see `scan_jobs::scan_library` for where
/// that ordering itself comes from) in order for `target`, stopping at the
/// first match; merges its fields into `item_id`'s `media_items` row and
/// saves its poster/backdrop.
///
/// `forced_provider`, when set (the metadata editor's per-item override —
/// see `lisek_db::MediaItemDetail`), replaces the whole `providers` list
/// with just that one provider's entry looked up out of it by key: a
/// forced provider that isn't currently enabled/configured (missing from
/// `providers` entirely) means this item is skipped this round rather than
/// silently falling back to the normal priority order, which would defeat
/// the point of forcing it in the first place. Individual `locked_fields`
/// protection (`overview` specifically) needs no parameter here at all —
/// `lisek_db::attach_provider_metadata` reads that column live off the row
/// it's updating; see that function's own doc comment.
///
/// `file_path` is the scanned video file's own on-disk path — used to
/// derive the media-folder destination (`poster.jpg`/`backdrop.jpg` beside
/// it) when `save_in_media_folder` is `true` (the library's own setting;
/// see `lisek_core::Library::save_metadata_in_media_folder`). `config_dir`
/// is always used as the local cache: either as the sole destination when
/// `save_in_media_folder` is `false`, or as the automatic fallback when a
/// media-folder write is denied. Either poster or backdrop falling back
/// (not simply "the setting was off") is persisted onto the item's
/// `metadata_fallback` flag via `lisek_db::set_metadata_fallback` — see that
/// function's own doc comment for why this is safe to call unconditionally
/// on every scan. `primary_language` is passed straight through to
/// whichever provider matches, as its own image-language priority (if it
/// has one) — currently the scanning admin's own UI language, pending a
/// dedicated server-wide metadata-language setting (see the roadmap).
///
/// A failed image download/write is logged and otherwise ignored — the
/// text metadata it's paired with is still worth keeping even if the
/// picture didn't come down this time. The returned [`EnrichOutcome`] is
/// how the caller (`scan_jobs::scan_library`) rolls per-item results up into a
/// scan-wide count instead of that information only ever existing as
/// scattered log lines — see that type's own doc comment for why this
/// exists.
///
/// `cache` shares lookups and downloads across the items of one pass; see
/// [`EnrichCache`].
#[allow(clippy::too_many_arguments)]
pub async fn enrich_item_cached(
    cache: &mut EnrichCache,
    pool: &PgPool,
    http: &reqwest::Client,
    config_dir: &Path,
    providers: &[(String, String)],
    item_id: Uuid,
    target: &EnrichTarget,
    file_path: &Path,
    save_in_media_folder: bool,
    primary_language: &str,
    forced_provider: Option<&str>,
) -> EnrichOutcome {
    let forced;
    let providers: &[(String, String)] = match forced_provider {
        Some(key) => {
            let Some(entry) = providers.iter().find(|(k, _)| k == key) else {
                tracing::warn!(
                    %item_id,
                    forced_provider = key,
                    "item has a forced provider that isn't currently enabled/configured — \
                     skipping enrichment this round rather than falling back to the priority list"
                );
                return EnrichOutcome::default();
            };
            forced = [entry.clone()];
            &forced
        }
        None => providers,
    };

    let memo_key = (target.clone(), forced_provider.map(str::to_owned));
    let found = match cache.lookups.get(&memo_key) {
        Some(found) => found.clone(),
        None => {
            let mut found = None;
            for (provider_key, api_key) in providers {
                if let Some(matched) =
                    try_provider(provider_key, api_key, target, primary_language).await
                {
                    found = Some((provider_key.clone(), matched));
                    break;
                }
            }
            if matches!(target, EnrichTarget::Series { .. }) {
                cache.lookups.insert(memo_key, found.clone());
            }
            found
        }
    };
    let Some((provider_key, matched)) = found else {
        tracing::debug!(%item_id, ?target, "no provider match");
        return EnrichOutcome::default();
    };
    let api_key = providers
        .iter()
        .find(|(key, _)| *key == provider_key)
        .map(|(_, api_key)| api_key.as_str())
        .unwrap_or_default();
    apply_match(
        cache,
        pool,
        http,
        config_dir,
        item_id,
        target,
        file_path,
        save_in_media_folder,
        &provider_key,
        api_key,
        primary_language,
        &matched,
    )
    .await
}

/// Saves one provider match onto an item: its fields (`provider_data`),
/// the collection it names (movies) and its poster/backdrop/logo, written
/// next to the video or into the local cache. Shared by enrichment and
/// "Fix match" (`identify`).
///
/// Episodes (Faza 10): the series' artwork goes into the series folder
/// (the parent of `Season 01/`, Jellyfin's layout), season posters next to
/// it as `season01-poster.jpg`; the episode's own title, synopsis, runtime,
/// score and still come from the provider's season (`api_key` and
/// `language` ask for it) and the still is kept in the local cache, so
/// season folders hold only videos.
#[allow(clippy::too_many_arguments)]
pub async fn apply_match(
    cache: &mut EnrichCache,
    pool: &PgPool,
    http: &reqwest::Client,
    config_dir: &Path,
    item_id: Uuid,
    target: &EnrichTarget,
    file_path: &Path,
    save_in_media_folder: bool,
    provider_key: &str,
    api_key: &str,
    language: &str,
    matched: &ProviderMatch,
) -> EnrichOutcome {
    tracing::debug!(
        %item_id,
        provider = provider_key,
        poster = matched.poster_url.is_some(),
        backdrop = matched.backdrop_url.is_some(),
        collection = matched.fields.get("collection").is_some_and(|c| !c.is_null()),
        "provider matched; saving metadata"
    );

    let is_series = matches!(target, EnrichTarget::Series { .. });
    let placement = if is_series {
        lisek_db::seasons::placement(pool, item_id).await.ok().flatten()
    } else {
        None
    };
    let mut fields = matched.fields.clone();
    let mut still_url = None;
    // Only an answer from the provider decides about the still; without one
    // the episode keeps what it has.
    let mut episode_known = false;
    if let Some(place) = &placement
        && let Some(episode) =
            episode_facts(cache, provider_key, api_key, language, &matched.fields, place).await
    {
        still_url = episode.get("still_url").and_then(|u| u.as_str()).map(str::to_owned);
        episode_known = true;
        fields["episode"] = episode;
    }
    // The still on disk came from this URL (read before the match below
    // replaces it).
    let previous_still = match &placement {
        Some(_) => lisek_db::seasons::episode_still_url(pool, item_id).await.ok().flatten(),
        None => None,
    };
    let mut problems = Vec::new();
    let metadata_saved =
        match lisek_db::attach_provider_metadata(pool, item_id, provider_key, &fields).await {
            Ok(()) => true,
            Err(err) => {
                tracing::error!(%item_id, provider_key, %err, "failed to save provider metadata");
                problems.push("metadata_save_failed".to_string());
                false
            }
        };
    // `episode_known` implies an episode (a placement).
    if episode_known {
        sync_episode_still(http, config_dir, item_id, previous_still, still_url.as_deref()).await;
    }
    if matches!(target, EnrichTarget::Movie { .. }) {
        sync_collection(cache, pool, http, config_dir, item_id, provider_key, &matched.fields)
            .await;
    }
    sync_people(cache, pool, http, config_dir, item_id, provider_key, matched.people.as_deref())
        .await;

    let mut used_fallback = false;
    let mut image_saved = false;
    let images: [(&Option<String>, &str, PathBuf, bool); 3] = [
        (&matched.poster_url, "poster.jpg", poster_cache_path(config_dir, item_id), true),
        (&matched.backdrop_url, "backdrop.jpg", backdrop_cache_path(config_dir, item_id), true),
        // A logo alone does not count as "an image was saved".
        (&matched.logo_url, "logo.png", logo_cache_path(config_dir, item_id), false),
    ];
    for (url, filename, fallback_dest, counts) in images {
        let Some(url) = url else { continue };
        let media_dest = if is_series {
            series_folder_destination(file_path, filename, save_in_media_folder)
        } else {
            media_folder_destination(file_path, filename, save_in_media_folder)
        };
        if media_dest.as_ref().is_some_and(|dest| cache.written.contains(dest)) {
            image_saved |= counts;
            continue;
        }
        tracing::debug!(%item_id, image = filename, url, "downloading image");
        let bytes = match cache.image(http, target, url).await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(%item_id, %err, image = filename, "failed to download image");
                problems.push(format!("{}_download_failed", image_kind(filename)));
                continue;
            }
        };
        match store_image(&bytes, media_dest.as_deref(), &fallback_dest).await {
            Ok(fell_back_to_cache) => {
                used_fallback |= fell_back_to_cache;
                image_saved |= counts;
                if let (false, Some(dest)) = (fell_back_to_cache, media_dest) {
                    cache.written.insert(dest);
                }
            }
            Err(err) => {
                tracing::warn!(%item_id, %err, image = filename, "failed to save image");
                problems.push(format!("{}_save_failed", image_kind(filename)));
            }
        }
    }

    if let Err(err) = lisek_db::set_metadata_fallback(pool, item_id, used_fallback).await {
        tracing::warn!(%item_id, %err, "failed to record metadata_fallback flag");
    }

    // After the series' own artwork is in the series folder: season
    // posters, and the old copies in season folders can be compared to it.
    if let Some(place) = &placement
        && cache.series_done.insert(place.series_id)
    {
        sync_series_layout(
            cache,
            pool,
            http,
            config_dir,
            place.series_id,
            target,
            file_path,
            save_in_media_folder,
            matched,
        )
        .await;
    }

    EnrichOutcome {
        matched: true,
        image_saved,
        metadata_saved,
        problems,
    }
}

/// `poster.jpg` -> `poster`.
fn image_kind(filename: &str) -> &str {
    filename.split('.').next().unwrap_or(filename)
}

/// Where a collection's own artwork is kept:
/// `config/metadata/collections/<id>/<file_name>`.
pub fn collection_image_path(config_dir: &Path, collection_id: Uuid, file_name: &str) -> PathBuf {
    config_dir
        .join("metadata")
        .join("collections")
        .join(collection_id.to_string())
        .join(file_name)
}

/// The collection a provider match names, as `fields.collection`:
/// `{"id": "10", "name": "...", "poster_url": ..., "backdrop_url": ...}`.
#[derive(Debug, PartialEq, Eq)]
enum CollectionField<'a> {
    /// No `collection` key: the provider could not tell (a failed lookup)
    /// — keep the film's current links.
    Unknown,
    /// `null`: the film belongs to no collection.
    None,
    Some {
        id: String,
        name: &'a str,
        poster_url: Option<&'a str>,
        backdrop_url: Option<&'a str>,
    },
}

fn collection_field(fields: &serde_json::Value) -> CollectionField<'_> {
    let Some(value) = fields.get("collection") else {
        return CollectionField::Unknown;
    };
    if value.is_null() {
        return CollectionField::None;
    }
    let id = match value.get("id") {
        Some(serde_json::Value::String(id)) if !id.is_empty() => id.clone(),
        Some(serde_json::Value::Number(id)) => id.to_string(),
        _ => return CollectionField::Unknown,
    };
    let Some(name) = value.get("name").and_then(|n| n.as_str()).filter(|n| !n.trim().is_empty())
    else {
        return CollectionField::Unknown;
    };
    let url = |key: &str| value.get(key).and_then(|u| u.as_str()).filter(|u| !u.is_empty());
    CollectionField::Some {
        id,
        name: name.trim(),
        poster_url: url("poster_url"),
        backdrop_url: url("backdrop_url"),
    }
}

/// Links a film to the collection its provider names (creating the
/// collection on first sight) and keeps the collection's artwork current.
/// Failures are logged; the film's own metadata is already saved.
async fn sync_collection(
    cache: &mut EnrichCache,
    pool: &PgPool,
    http: &reqwest::Client,
    config_dir: &Path,
    item_id: Uuid,
    provider_key: &str,
    fields: &serde_json::Value,
) {
    let (provider_id, name, poster_url, backdrop_url) = match collection_field(fields) {
        CollectionField::Unknown => return,
        CollectionField::None => {
            if let Err(err) = lisek_db::collections::unlink_item(pool, item_id, provider_key).await {
                tracing::warn!(%item_id, %err, "failed to remove a film from its collection");
            }
            return;
        }
        CollectionField::Some {
            id,
            name,
            poster_url,
            backdrop_url,
        } => (id, name, poster_url, backdrop_url),
    };
    let stored = match lisek_db::collections::upsert(pool, provider_key, &provider_id, name).await {
        Ok(stored) => stored,
        Err(err) => {
            tracing::warn!(%item_id, %err, collection = name, "failed to save collection");
            return;
        }
    };
    tracing::debug!(
        %item_id,
        collection_id = %stored.id,
        collection = name,
        provider_id,
        "linking film to collection"
    );
    if let Err(err) = lisek_db::collections::link_item(pool, stored.id, item_id).await {
        tracing::warn!(%item_id, %err, collection = name, "failed to link film to collection");
    }
    if !cache.collections_done.insert(stored.id) {
        return;
    }

    let poster = sync_artwork(
        http,
        &collection_image_path(config_dir, stored.id, "poster.jpg"),
        name,
        stored.poster_url.clone(),
        poster_url,
    )
    .await;
    let backdrop = sync_artwork(
        http,
        &collection_image_path(config_dir, stored.id, "backdrop.jpg"),
        name,
        stored.backdrop_url.clone(),
        backdrop_url,
    )
    .await;
    if (&poster, &backdrop) != (&stored.poster_url, &stored.backdrop_url)
        && let Err(err) = lisek_db::collections::set_artwork(
            pool,
            stored.id,
            poster.as_deref(),
            backdrop.as_deref(),
        )
        .await
    {
        tracing::warn!(collection = name, %err, "failed to record collection artwork");
    }
}

/// Brings one stored image (a collection's poster or backdrop, a person's
/// profile photo) on disk at `path` in line with the provider: keeps an
/// up-to-date file, downloads a new or changed one, removes one the
/// provider dropped. Returns the source URL now on disk. `label` only
/// names the image in the logs.
///
/// No `.await` inside a `match` guard here, on purpose. rustc 1.97
/// miscompiles `match array[i] { Some(x) if cond(x).await => .., Some(x)
/// => .. }` in an async fn: the element address is computed from an index
/// slot on the stack that is never written. In debug builds the first
/// image of the first collection read through a garbage index — on
/// Windows that crashed the whole server with STATUS_ACCESS_VIOLATION
/// right after a scan started (`artwork_sync_survives_a_dirty_stack`
/// guards against it coming back).
pub(crate) async fn sync_artwork(
    http: &reqwest::Client,
    path: &Path,
    label: &str,
    stored_url: Option<String>,
    wanted_url: Option<&str>,
) -> Option<String> {
    let Some(url) = wanted_url else {
        if stored_url.is_some() {
            tracing::debug!(label, path = %path.display(), "provider dropped this image");
        }
        let _ = tokio::fs::remove_file(path).await;
        return None;
    };
    let unchanged = stored_url.as_deref() == Some(url);
    if unchanged && tokio::fs::try_exists(path).await.unwrap_or(false) {
        return stored_url;
    }
    tracing::debug!(label, url, path = %path.display(), "downloading artwork");
    match download_image(http, url, path).await {
        Ok(()) => Some(url.to_owned()),
        Err(err) => {
            tracing::warn!(label, %err, path = %path.display(), "failed to download artwork");
            // Whatever was on disk before is still there.
            stored_url
        }
    }
}

/// Where a person's profile photo is kept:
/// `config/metadata/people/<id>/profile.jpg`.
pub fn person_image_path(config_dir: &Path, person_id: Uuid) -> PathBuf {
    config_dir
        .join("metadata")
        .join("people")
        .join(person_id.to_string())
        .join("profile.jpg")
}

/// The URL to compare against the stored one: TMDB serves one picture in
/// several sizes (`/t/p/w185/x.jpg`, `/t/p/h632/x.jpg`), and the person
/// page may have stored a larger size of the same picture the credits now
/// name — that is no reason to download it again.
fn same_picture_or<'a>(stored: Option<&'a str>, wanted: Option<&'a str>) -> Option<&'a str> {
    let file = |url: &'a str| url.rsplit('/').next().filter(|f| !f.is_empty());
    match (stored, wanted) {
        (Some(stored), Some(wanted)) if file(stored).is_some() && file(stored) == file(wanted) => {
            Some(stored)
        }
        (_, wanted) => wanted,
    }
}

/// Stores the item's cast and crew (only when the provider answered; TMDB
/// ids only, the one provider with people) and keeps each person's profile
/// photo current. Failures are logged; the item's own metadata is saved.
async fn sync_people(
    cache: &mut EnrichCache,
    pool: &PgPool,
    http: &reqwest::Client,
    config_dir: &Path,
    item_id: Uuid,
    provider_key: &str,
    people: Option<&[ProviderPerson]>,
) {
    let Some(people) = people else { return };
    if provider_key != "tmdb" {
        return;
    }
    let credits: Vec<lisek_db::people::NewCredit> = people
        .iter()
        .filter_map(|p| {
            Some(lisek_db::people::NewCredit {
                tmdb_id: p.provider_id.parse().ok()?,
                name: p.name.clone(),
                role: p.role.as_str().to_string(),
                character: p.character.clone(),
                order: p.order,
            })
        })
        .collect();
    let linked = match lisek_db::people::replace_credits(pool, item_id, &credits).await {
        Ok(linked) => linked,
        Err(err) => {
            tracing::warn!(%item_id, %err, "failed to save cast and crew");
            return;
        }
    };
    tracing::debug!(%item_id, credits = credits.len(), people = linked.len(), "cast and crew saved");
    let photos: HashMap<i64, &str> = people
        .iter()
        .filter_map(|p| Some((p.provider_id.parse().ok()?, p.profile_url.as_deref()?)))
        .collect();
    for person in linked {
        if !cache.people_done.insert(person.id) {
            continue;
        }
        let wanted = photos.get(&person.tmdb_id).copied();
        let wanted = same_picture_or(person.profile_url.as_deref(), wanted);
        let path = person_image_path(config_dir, person.id);
        let label = person.tmdb_id.to_string();
        let now = sync_artwork(http, &path, &label, person.profile_url.clone(), wanted).await;
        if now != person.profile_url
            && let Err(err) = lisek_db::people::set_profile_url(pool, person.id, now.as_deref()).await
        {
            tracing::warn!(person_id = %person.id, %err, "failed to record a profile photo");
        }
    }
}

/// Deletes people no title credits any more, with their photos, plus photo
/// folders no person owns.
pub async fn prune_people(pool: &PgPool, config_dir: &Path) {
    if let Err(err) = lisek_db::people::prune_orphans(pool).await {
        tracing::warn!(%err, "failed to remove uncredited people");
        return;
    }
    let Ok(ids) = lisek_db::people::all_ids(pool).await else {
        return;
    };
    let keep: HashSet<String> = ids.into_iter().map(|id| id.to_string()).collect();
    let root = config_dir.join("metadata").join("people");
    let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if !keep.contains(entry.file_name().to_string_lossy().as_ref()) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

/// Deletes collections left without films (after a scan or refresh) and
/// their artwork, plus artwork folders no collection owns.
pub async fn prune_collections(pool: &PgPool, config_dir: &Path) {
    // Credits go with deleted items too; sweep their people in the same pass.
    prune_people(pool, config_dir).await;
    if let Err(err) = lisek_db::collections::prune_empty(pool).await {
        tracing::warn!(%err, "failed to remove empty collections");
        return;
    }
    let Ok(ids) = lisek_db::collections::all_ids(pool).await else {
        return;
    };
    let keep: HashSet<String> = ids.into_iter().map(|id| id.to_string()).collect();
    let root = config_dir.join("metadata").join("collections");
    let Ok(mut entries) = tokio::fs::read_dir(&root).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if !keep.contains(entry.file_name().to_string_lossy().as_ref()) {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnrichOutcome {
    /// A provider (any of them, in priority order) returned a match for
    /// this item.
    pub matched: bool,
    /// At least one of poster/backdrop was actually written to disk —
    /// media folder or local cache, either counts. `false` while `matched`
    /// is `true` means a real title match with no usable provider image,
    /// or every image download/write failing; see the per-item
    /// `tracing::warn!` lines this function emits for which one happened.
    pub image_saved: bool,
    /// `provider_data` was written (Fix match reports a failure here
    /// instead of answering "done").
    pub metadata_saved: bool,
    /// What went wrong on the way, as stable codes for the admin
    /// (`poster_download_failed`, `backdrop_save_failed`, ...).
    pub problems: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_field_tells_unknown_from_none() {
        assert_eq!(
            collection_field(&serde_json::json!({"overview": "x"})),
            CollectionField::Unknown
        );
        assert_eq!(
            collection_field(&serde_json::json!({"collection": null})),
            CollectionField::None
        );
        assert_eq!(
            collection_field(&serde_json::json!({"collection": {"id": 10, "name": " Star Wars "}})),
            CollectionField::Some {
                id: "10".to_string(),
                name: "Star Wars",
                poster_url: None,
                backdrop_url: None,
            }
        );
        assert_eq!(
            collection_field(&serde_json::json!({"collection": {"id": "10", "name": ""}})),
            CollectionField::Unknown,
            "a nameless collection is ignored, not used to unlink"
        );
        assert_eq!(
            collection_field(&serde_json::json!({
                "collection": {"id": "10", "name": "SW", "poster_url": "https://x/p.jpg", "backdrop_url": ""}
            })),
            CollectionField::Some {
                id: "10".to_string(),
                name: "SW",
                poster_url: Some("https://x/p.jpg"),
                backdrop_url: None,
            }
        );
    }

    /// Fills a stretch of stack with a non-zero pattern, so a read from a
    /// never-written stack slot gets garbage instead of a lucky zero.
    #[inline(never)]
    fn dirty_stack() {
        let mut buf = [0xABu8; 128 * 1024];
        std::hint::black_box(&mut buf);
    }

    /// Regression test for the rustc miscompilation described on
    /// [`sync_artwork`]: with the old `match wanted[slot] { .. if ...await }`
    /// loop this test process crashed (SIGSEGV / STATUS_ACCESS_VIOLATION).
    #[test]
    fn artwork_sync_survives_a_dirty_stack() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let dir = std::env::temp_dir().join(format!("lisek-artwork-{}", Uuid::new_v4()));
        let id = Uuid::new_v4();
        let http = reqwest::Client::new();
        let poster = collection_image_path(&dir, id, "poster.jpg");
        std::fs::create_dir_all(poster.parent().unwrap()).unwrap();
        std::fs::write(&poster, b"jpeg").unwrap();
        dirty_stack();
        runtime.block_on(async {
            dirty_stack();
            let url = "http://127.0.0.1:9/poster.jpg";
            // Unchanged URL and the file is there: kept, nothing downloaded.
            let kept = sync_artwork(&http, &poster, "Saga", Some(url.into()), Some(url)).await;
            assert_eq!(kept.as_deref(), Some(url));
            // A new URL that cannot be downloaded: the old one stays.
            let failed = sync_artwork(
                &http, &poster, "Saga", Some(url.into()), Some("http://127.0.0.1:9/new.jpg"),
            )
            .await;
            assert_eq!(failed.as_deref(), Some(url));
            // The provider dropped the image: file and URL go.
            let dropped = sync_artwork(&http, &poster, "Saga", Some(url.into()), None).await;
            assert_eq!(dropped, None);
        });
        assert!(!poster.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A tiny HTTP server answering every request with the same bytes,
    /// counting requests.
    async fn image_server() -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut buf = [0u8; 2048];
                let _ = socket.read(&mut buf).await;
                let body = b"jpeg";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body).await;
            }
        });
        (base, hits)
    }

    /// End to end against a real database (opt-in, `LISEK_TEST_DATABASE_URL`):
    /// credits saved, photos downloaded once, a larger stored size of the
    /// same picture kept, `None` keeps the cast, an empty list clears it and
    /// the sweep removes people with their photos.
    #[tokio::test]
    async fn people_are_saved_with_their_photos_once() {
        use lisek_plugin_api::PersonRole;
        use sqlx::{AssertSqlSafe, ConnectOptions};
        use std::sync::atomic::Ordering;
        let Ok(url) = std::env::var("LISEK_TEST_DATABASE_URL") else {
            eprintln!("LISEK_TEST_DATABASE_URL not set; skipping database test");
            return;
        };
        let admin_options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin_options.clone())
            .await
            .unwrap();
        let name = format!("lisek_test_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_with(admin_options.database(&name).disable_statement_logging())
            .await
            .unwrap();
        lisek_db::run_migrations(&pool).await.unwrap();

        let library = lisek_db::create_library(&pool, "Filmy", "movies", &["/media/Filmy".into()], false)
            .await
            .unwrap()
            .id;
        lisek_db::persist_scan_results(
            &pool,
            library,
            &[lisek_library::ScanResult::Movie(lisek_library::ScannedMovie {
                file_path: PathBuf::from("/media/Filmy/Star Wars.mkv"),
                title: "Star Wars".into(),
                year: Some(1977),
                file: lisek_library::FileFacts {
                    size_bytes: Some(1),
                    modified: None,
                },
            })],
        )
        .await
        .unwrap();
        let (item_id,): (Uuid,) = sqlx::query_as("SELECT id FROM media_items").fetch_one(&pool).await.unwrap();

        let (base, hits) = image_server().await;
        let small = format!("{base}/t/p/w185/hf.jpg");
        let people = vec![
            ProviderPerson {
                provider_id: "3".into(),
                name: "Harrison Ford".into(),
                role: PersonRole::Actor,
                character: Some("Han Solo".into()),
                order: 0,
                profile_url: Some(small.clone()),
            },
            ProviderPerson {
                provider_id: "1".into(),
                name: "George Lucas".into(),
                role: PersonRole::Director,
                character: None,
                order: 0,
                profile_url: None,
            },
        ];
        let config = std::env::temp_dir().join(format!("lisek-people-{}", Uuid::new_v4()));
        let http = reqwest::Client::new();
        let sync = |people: Option<Vec<ProviderPerson>>| {
            let (pool, http, config) = (pool.clone(), http.clone(), config.clone());
            async move {
                let mut cache = EnrichCache::default();
                sync_people(&mut cache, &pool, &http, &config, item_id, "tmdb", people.as_deref()).await;
            }
        };

        sync(Some(people.clone())).await;
        let credits = lisek_db::people::item_credits(&pool, item_id, 10).await.unwrap();
        assert_eq!(
            credits.iter().map(|c| (c.name.as_str(), c.has_image)).collect::<Vec<_>>(),
            [("George Lucas", false), ("Harrison Ford", true)]
        );
        let ford = credits[1].person_id;
        let photo = person_image_path(&config, ford);
        assert_eq!(std::fs::read(&photo).unwrap(), b"jpeg");
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // Again (the six-hourly refresh): nothing downloaded.
        sync(Some(people.clone())).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        // The person page stored a bigger size of the same picture: kept.
        let large = format!("{base}/t/p/h632/hf.jpg");
        lisek_db::people::set_profile_url(&pool, ford, Some(&large)).await.unwrap();
        sync(Some(people.clone())).await;
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let (stored,): (Option<String>,) = sqlx::query_as("SELECT profile_url FROM people WHERE id = $1")
            .bind(ford)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, Some(large));

        // Another provider, or no answer: the cast stays.
        let mut cache = EnrichCache::default();
        sync_people(&mut cache, &pool, &http, &config, item_id, "tvdb", Some(&[])).await;
        sync(None).await;
        assert_eq!(lisek_db::people::item_credits(&pool, item_id, 10).await.unwrap().len(), 2);

        // Nobody credited any more: the sweep removes people and photos.
        sync(Some(Vec::new())).await;
        assert!(lisek_db::people::item_credits(&pool, item_id, 10).await.unwrap().is_empty());
        std::fs::create_dir_all(config.join("metadata/people/stray")).unwrap();
        prune_people(&pool, &config).await;
        assert!(lisek_db::people::all_ids(&pool).await.unwrap().is_empty());
        assert!(!photo.exists());
        assert!(!config.join("metadata/people/stray").exists());

        pool.close().await;
        sqlx::query(AssertSqlSafe(format!("DROP DATABASE {name} WITH (FORCE)")))
            .execute(&admin)
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&config);
    }

    /// Faza 10, against a real database (opt-in) and a real folder: the
    /// series' artwork lands in the series folder, season posters next to
    /// it as `season01-poster.jpg`, the old copy in `Season 01/` goes (an
    /// administrator's own different file stays), and an unchanged season
    /// poster is not downloaded again.
    #[tokio::test]
    async fn series_artwork_goes_to_the_series_folder() {
        use lisek_plugin_api::ProviderSeason;
        use sqlx::{AssertSqlSafe, ConnectOptions};
        use std::sync::atomic::Ordering;
        let Ok(url) = std::env::var("LISEK_TEST_DATABASE_URL") else {
            eprintln!("LISEK_TEST_DATABASE_URL not set; skipping database test");
            return;
        };
        let admin_options: sqlx::postgres::PgConnectOptions = url.parse().unwrap();
        let admin = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin_options.clone())
            .await
            .unwrap();
        let name = format!("lisek_test_{}", Uuid::new_v4().simple());
        sqlx::query(AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_with(admin_options.database(&name).disable_statement_logging())
            .await
            .unwrap();
        lisek_db::run_migrations(&pool).await.unwrap();

        let root = std::env::temp_dir().join(format!("lisek-series-{}", Uuid::new_v4()));
        let series_dir = root.join("Shows").join("13 Posterunek (1997)");
        let season_dir = series_dir.join("Season 01");
        std::fs::create_dir_all(&season_dir).unwrap();
        std::fs::write(season_dir.join("poster.jpg"), b"jpeg").unwrap(); // our old copy
        std::fs::write(season_dir.join("backdrop.jpg"), b"admin's own").unwrap();
        let library = lisek_db::create_library(
            &pool,
            "Seriale",
            "tv_shows",
            &[root.join("Shows").to_string_lossy().into_owned()],
            true,
        )
        .await
        .unwrap()
        .id;
        let episodes: Vec<lisek_library::ScanResult> = (1..=2)
            .map(|n| {
                let file = season_dir.join(format!("S01E0{n}.mkv"));
                std::fs::write(&file, b"video").unwrap();
                lisek_library::ScanResult::Episode(lisek_library::ScannedEpisode {
                    file_path: file,
                    series_title: "13 Posterunek".into(),
                    season: 1,
                    episode: n,
                    end_episode: None,
                    file: lisek_library::FileFacts {
                        size_bytes: Some(5),
                        modified: None,
                    },
                })
            })
            .collect();
        lisek_db::persist_scan_results(&pool, library, &episodes).await.unwrap();
        let ids: Vec<(Uuid, String)> =
            sqlx::query_as("SELECT id, file_path FROM media_items ORDER BY file_path")
                .fetch_all(&pool)
                .await
                .unwrap();

        let (base, hits) = image_server().await;
        let matched = ProviderMatch {
            fields: serde_json::json!({"tmdb_id": 47999, "overview": "x"}),
            poster_url: Some(format!("{base}/poster.jpg")),
            backdrop_url: Some(format!("{base}/backdrop.jpg")),
            seasons: vec![
                ProviderSeason {
                    number: 1,
                    name: Some("Sezon 1".into()),
                    poster_url: Some(format!("{base}/s1.jpg")),
                    ..ProviderSeason::default()
                },
                ProviderSeason { number: 2, name: Some("Sezon 2".into()), ..ProviderSeason::default() },
            ],
            ..ProviderMatch::default()
        };
        let target = EnrichTarget::Series { title: "13 Posterunek".into() };
        let config = root.join("config");
        let http = reqwest::Client::new();
        let pass = |cache: &mut EnrichCache| {
            let (pool, http, config, ids, target, matched) =
                (pool.clone(), http.clone(), config.clone(), ids.clone(), target.clone(), matched.clone());
            let cache = std::mem::take(cache);
            async move {
                let mut cache = cache;
                for (id, file) in &ids {
                    let outcome = apply_match(
                        &mut cache, &pool, &http, &config, *id, &target, Path::new(file), true,
                        "test", "", "pl", &matched,
                    )
                    .await;
                    assert!(outcome.image_saved);
                }
            }
        };
        pass(&mut EnrichCache::default()).await;

        let mut in_series: Vec<String> = std::fs::read_dir(&series_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        in_series.sort();
        assert_eq!(in_series, ["Season 01", "backdrop.jpg", "poster.jpg", "season01-poster.jpg"]);
        let mut in_season: Vec<String> = std::fs::read_dir(&season_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        in_season.sort();
        assert_eq!(in_season, ["S01E01.mkv", "S01E02.mkv", "backdrop.jpg"], "only the admin's own file stays");
        assert_eq!(std::fs::read(season_dir.join("backdrop.jpg")).unwrap(), b"admin's own");
        // poster + backdrop once for both episodes, season poster once.
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        let cards = lisek_db::seasons::season_cards(&pool, ids[0].0, Uuid::new_v4()).await.unwrap();
        assert_eq!((cards[0].name.as_deref(), cards[0].has_poster), (Some("Sezon 1"), true));

        // The next pass: series images are written again, the season poster is not.
        pass(&mut EnrichCache::default()).await;
        assert_eq!(hits.load(Ordering::SeqCst), 5);

        pool.close().await;
        sqlx::query(AssertSqlSafe(format!("DROP DATABASE {name} WITH (FORCE)")))
            .execute(&admin)
            .await
            .unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bigger_size_of_the_same_picture_counts_as_unchanged() {
        let small = "https://image.tmdb.org/t/p/w185/abc.jpg";
        let large = "https://image.tmdb.org/t/p/h632/abc.jpg";
        let other = "https://image.tmdb.org/t/p/w185/xyz.jpg";
        assert_eq!(same_picture_or(Some(large), Some(small)), Some(large));
        assert_eq!(same_picture_or(Some(large), Some(other)), Some(other));
        assert_eq!(same_picture_or(None, Some(small)), Some(small));
        assert_eq!(same_picture_or(Some(large), None), None);
        assert_eq!(
            person_image_path(Path::new("/cfg"), Uuid::nil()),
            Path::new("/cfg/metadata/people/00000000-0000-0000-0000-000000000000/profile.jpg")
        );
    }

    #[test]
    fn collection_artwork_lives_under_the_config_dir() {
        let id = Uuid::nil();
        assert_eq!(
            collection_image_path(Path::new("/cfg"), id, "poster.jpg"),
            Path::new("/cfg/metadata/collections/00000000-0000-0000-0000-000000000000/poster.jpg")
        );
    }
}
