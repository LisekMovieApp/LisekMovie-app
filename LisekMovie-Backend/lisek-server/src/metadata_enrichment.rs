//! Fills in a freshly-scanned `media_items` row with real metadata: tries
//! every enabled, configured provider in the admin's priority order (see
//! `lisek_db::list_enabled_provider_keys`) until one returns a match, merges
//! its fields into the row, then saves its poster/backdrop image.
//!
//! As of Faza 5 round 2, "saves its poster/backdrop image" means writing
//! directly next to the video file by default: `poster.jpg`/
//! `backdrop.jpg` (see
//! [`media_folder_destination`]), with the app's own local `config_dir`
//! cache used either as the library's deliberate choice (`save_in_media_folder
//! == false`) or as an automatic fallback when the media-folder write itself
//! is denied (see `lisek_metadata::image_cache::save_image`) — the latter
//! case gets persisted onto the item's `metadata_fallback` flag so a future
//! Admin Panel warning view can list it. For a TMDB match, the image itself
//! is picked via the primary-language → English → untagged → any-available
//! chain (see `lisek_metadata::tmdb::TmdbClient::movie_images`/
//! `series_images`) rather than TMDB's own single "default" search-result
//! image, which is what a bare `/search/movie` response returns and is not
//! language-prioritized at all.
//!
//! Called once per item from `admin::scan_library`, right after
//! `lisek_db::persist_scan_results`. A provider failing (network error,
//! rejected key, no match) for one item is logged and treated as "try the
//! next provider" / "leave this item without provider metadata for now" —
//! it never aborts the rest of the scan, since one bad provider or one
//! unmatched title shouldn't cost the admin every other item's data.

use std::path::{Path, PathBuf};

use lisek_db::{EnrichTarget, PgPool};
use lisek_metadata::image_cache::{backdrop_cache_path, poster_cache_path, save_image};
use lisek_metadata::tmdb::{self, TmdbImages};
use lisek_metadata::tvdb;
use uuid::Uuid;

/// One provider match's fields, plus optional ready-to-download poster/
/// backdrop URLs — [`tmdb`]'s bare image *paths* and [`tvdb`]'s one
/// ready-made image URL both get normalized to this same shape here, so
/// the rest of this module doesn't need to know which provider produced
/// them.
struct MatchedMetadata {
    fields: serde_json::Value,
    poster_url: Option<String>,
    backdrop_url: Option<String>,
}

/// Fetches a TMDB match's language-prioritized images and falls back to the
/// search result's own (non-prioritized) `poster_path`/`backdrop_path` when
/// the extra `/images` request itself fails — a title match is still worth
/// keeping even if this one follow-up call didn't succeed, and TMDB's
/// search-result image is still a real (if not language-optimal) picture
/// rather than nothing at all.
fn tmdb_images_or_fallback(
    result: Result<TmdbImages, tmdb::TmdbError>,
    title: &str,
    fallback_poster: Option<&str>,
    fallback_backdrop: Option<&str>,
) -> (Option<String>, Option<String>) {
    let images = match result {
        Ok(images) => images,
        Err(err) => {
            tracing::warn!(
                provider = "tmdb",
                %err,
                title,
                "language-prioritized image lookup failed — using the search result's default image instead"
            );
            TmdbImages::default()
        }
    };
    (
        images
            .poster_path
            .or_else(|| fallback_poster.map(str::to_string)),
        images
            .backdrop_path
            .or_else(|| fallback_backdrop.map(str::to_string)),
    )
}

async fn try_provider(
    provider_key: &str,
    api_key: &str,
    target: &EnrichTarget,
    primary_language: &str,
) -> Option<MatchedMetadata> {
    match (provider_key, target) {
        ("tmdb", EnrichTarget::Movie { title, year }) => {
            let client = tmdb::TmdbClient::new(api_key);
            match client.search_movie(title, *year).await {
                Ok(Some(m)) => {
                    let images_result = client.movie_images(m.tmdb_id, primary_language).await;
                    let (poster_path, backdrop_path) = tmdb_images_or_fallback(
                        images_result,
                        title,
                        m.poster_path.as_deref(),
                        m.backdrop_path.as_deref(),
                    );
                    Some(MatchedMetadata {
                        poster_url: poster_path.as_deref().map(|p| tmdb::image_url(p, "w500")),
                        backdrop_url: backdrop_path
                            .as_deref()
                            .map(|p| tmdb::image_url(p, "w1280")),
                        fields: serde_json::json!({
                            "tmdb_id": m.tmdb_id,
                            "overview": m.overview,
                        }),
                    })
                }
                Ok(None) => None,
                Err(err) => {
                    tracing::warn!(provider = "tmdb", %err, title, "movie metadata lookup failed");
                    None
                }
            }
        }
        ("tmdb", EnrichTarget::Series { title }) => {
            let client = tmdb::TmdbClient::new(api_key);
            match client.search_series(title).await {
                Ok(Some(m)) => {
                    let images_result = client.series_images(m.tmdb_id, primary_language).await;
                    let (poster_path, backdrop_path) = tmdb_images_or_fallback(
                        images_result,
                        title,
                        m.poster_path.as_deref(),
                        m.backdrop_path.as_deref(),
                    );
                    Some(MatchedMetadata {
                        poster_url: poster_path.as_deref().map(|p| tmdb::image_url(p, "w500")),
                        backdrop_url: backdrop_path
                            .as_deref()
                            .map(|p| tmdb::image_url(p, "w1280")),
                        fields: serde_json::json!({
                            "tmdb_id": m.tmdb_id,
                            "overview": m.overview,
                        }),
                    })
                }
                Ok(None) => None,
                Err(err) => {
                    tracing::warn!(provider = "tmdb", %err, title, "series metadata lookup failed");
                    None
                }
            }
        }
        ("tvdb", EnrichTarget::Series { title }) => {
            let client = tvdb::TvdbClient::new(api_key);
            match client.search_series(title).await {
                // TVDB's search response carries one thumbnail URL, not a
                // separate poster/backdrop pair — used as the poster; no
                // backdrop is cached for a TVDB-only match. TVDB has no
                // per-language `/images` endpoint equivalent to TMDB's in
                // this client, so no language-priority chain applies here.
                Ok(Some(m)) => Some(MatchedMetadata {
                    poster_url: m.image_url.clone(),
                    backdrop_url: None,
                    fields: serde_json::json!({
                        "tvdb_id": m.tvdb_id,
                        "overview": m.overview,
                    }),
                }),
                Ok(None) => None,
                Err(err) => {
                    tracing::warn!(provider = "tvdb", %err, title, "series metadata lookup failed");
                    None
                }
            }
        }
        // TVDB's v4 movie search exists but isn't wired up in this pass —
        // TMDB alone covers movies for now (see the crate doc comment on
        // `lisek-metadata` for the built-in-provider scope).
        ("tvdb", EnrichTarget::Movie { .. }) => None,
        (unknown, _) => {
            tracing::warn!(
                provider = unknown,
                "no metadata client for this provider key"
            );
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

/// Tries `providers` (an admin-priority-ordered `(provider_key, api_key)`
/// list) in order for `target`, stopping at the first match; merges its
/// fields into `item_id`'s `media_items` row and saves its poster/backdrop.
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
/// on every scan. `primary_language` is the TMDB image-language chain's
/// first choice (see `lisek_metadata::tmdb::TmdbClient::movie_images`) —
/// currently the scanning admin's own UI language, pending a dedicated
/// server-wide metadata-language setting (see the roadmap).
///
/// A failed image download/write is logged and otherwise ignored — the
/// text metadata it's paired with is still worth keeping even if the
/// picture didn't come down this time. The returned [`EnrichOutcome`] is
/// how the caller (`admin::scan_library`) rolls per-item results up into a
/// scan-wide count instead of that information only ever existing as
/// scattered log lines — see that type's own doc comment for why this
/// exists.
#[allow(clippy::too_many_arguments)]
pub async fn enrich_item(
    pool: &PgPool,
    http: &reqwest::Client,
    config_dir: &Path,
    providers: &[(String, String)],
    item_id: Uuid,
    target: &EnrichTarget,
    file_path: &Path,
    save_in_media_folder: bool,
    primary_language: &str,
) -> EnrichOutcome {
    for (provider_key, api_key) in providers {
        let Some(matched) = try_provider(provider_key, api_key, target, primary_language).await
        else {
            continue;
        };

        if let Err(err) =
            lisek_db::attach_provider_metadata(pool, item_id, provider_key, &matched.fields).await
        {
            tracing::warn!(%item_id, provider_key, %err, "failed to save provider metadata");
        }

        let mut used_fallback = false;
        let mut image_saved = false;

        if let Some(url) = &matched.poster_url {
            let media_dest =
                media_folder_destination(file_path, "poster.jpg", save_in_media_folder);
            let fallback_dest = poster_cache_path(config_dir, item_id);
            match save_image(http, url, media_dest.as_deref(), &fallback_dest).await {
                Ok(fell_back_to_cache) => {
                    used_fallback |= fell_back_to_cache;
                    image_saved = true;
                }
                Err(err) => tracing::warn!(%item_id, %err, "failed to save poster image"),
            }
        }
        if let Some(url) = &matched.backdrop_url {
            let media_dest =
                media_folder_destination(file_path, "backdrop.jpg", save_in_media_folder);
            let fallback_dest = backdrop_cache_path(config_dir, item_id);
            match save_image(http, url, media_dest.as_deref(), &fallback_dest).await {
                Ok(fell_back_to_cache) => {
                    used_fallback |= fell_back_to_cache;
                    image_saved = true;
                }
                Err(err) => tracing::warn!(%item_id, %err, "failed to save backdrop image"),
            }
        }

        if let Err(err) = lisek_db::set_metadata_fallback(pool, item_id, used_fallback).await {
            tracing::warn!(%item_id, %err, "failed to record metadata_fallback flag");
        }

        return EnrichOutcome {
            matched: true,
            image_saved,
        };
    }

    EnrichOutcome::default()
}

/// What happened when one item was run through the provider chain — lets
/// `admin::scan_library` aggregate a scan-wide "N/total items got a
/// picture" count for its response, so a provider that's misconfigured (no
/// API key set, or one TMDB rejects) or simply finds nothing shows up as a
/// number the admin sees immediately after clicking "scan" instead of only
/// as a 404 in the browser console once they open the library — the same
/// gap Faza 5 round 2's hardening pass closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EnrichOutcome {
    /// A provider (any of them, in priority order) returned a match for
    /// this item.
    pub matched: bool,
    /// At least one of poster/backdrop was actually written to disk —
    /// media folder or local cache, either counts. `false` while `matched`
    /// is `true` means a real title match with no usable TMDB/TVDB image,
    /// or every image download/write failing; see the per-item
    /// `tracing::warn!` lines this function emits for which one happened.
    pub image_saved: bool,
}
