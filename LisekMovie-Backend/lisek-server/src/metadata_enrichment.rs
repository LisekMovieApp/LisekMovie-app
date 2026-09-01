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
//! Admin Panel warning view can list it. Which image a provider hands back
//! (language-prioritized or not) is entirely that provider extension's own
//! concern as of Faza 7 — this module just downloads and writes whatever
//! `poster_url`/`backdrop_url` it's given, the same way for every provider.
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
use lisek_plugin_api::{MovieQuery, ProviderMatch, SeriesQuery};
use uuid::Uuid;

use crate::provider_registry;

/// Builds `provider_key`'s extension (see [`provider_registry`]) and asks
/// it for `target`, dispatching to `find_movie`/`find_series` depending on
/// which variant `target` is. `None` covers an unregistered `provider_key`,
/// no match, and a failed lookup alike — all three are "try the next
/// provider in the admin's priority list" as far as [`enrich_item`] is
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
        Ok(matched) => matched,
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
/// on every scan. `primary_language` is passed straight through to
/// whichever provider matches, as its own image-language priority (if it
/// has one) — currently the scanning admin's own UI language, pending a
/// dedicated server-wide metadata-language setting (see the roadmap).
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
    /// is `true` means a real title match with no usable provider image,
    /// or every image download/write failing; see the per-item
    /// `tracing::warn!` lines this function emits for which one happened.
    pub image_saved: bool,
}
