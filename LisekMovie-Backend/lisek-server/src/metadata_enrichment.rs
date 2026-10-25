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
use lisek_plugin_api::{MovieQuery, ProviderMatch, SeriesQuery};
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
    tracing::debug!(
        %item_id,
        provider = provider_key,
        poster = matched.poster_url.is_some(),
        backdrop = matched.backdrop_url.is_some(),
        collection = matched.fields.get("collection").is_some_and(|c| !c.is_null()),
        "provider matched; saving metadata"
    );

    if let Err(err) =
        lisek_db::attach_provider_metadata(pool, item_id, &provider_key, &matched.fields).await
    {
        tracing::warn!(%item_id, provider_key, %err, "failed to save provider metadata");
    }
    if matches!(target, EnrichTarget::Movie { .. }) {
        sync_collection(cache, pool, http, config_dir, item_id, &provider_key, &matched.fields)
            .await;
    }

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
        let media_dest = media_folder_destination(file_path, filename, save_in_media_folder);
        if media_dest.as_ref().is_some_and(|dest| cache.written.contains(dest)) {
            image_saved |= counts;
            continue;
        }
        tracing::debug!(%item_id, image = filename, url, "downloading image");
        let bytes = match cache.image(http, target, url).await {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(%item_id, %err, image = filename, "failed to download image");
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
            Err(err) => tracing::warn!(%item_id, %err, image = filename, "failed to save image"),
        }
    }

    if let Err(err) = lisek_db::set_metadata_fallback(pool, item_id, used_fallback).await {
        tracing::warn!(%item_id, %err, "failed to record metadata_fallback flag");
    }

    EnrichOutcome {
        matched: true,
        image_saved,
    }
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
        config_dir,
        stored.id,
        name,
        "poster.jpg",
        stored.poster_url.clone(),
        poster_url,
    )
    .await;
    let backdrop = sync_artwork(
        http,
        config_dir,
        stored.id,
        name,
        "backdrop.jpg",
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

/// Brings one collection image on disk in line with the provider: keeps an
/// up-to-date file, downloads a new or changed one, removes one the
/// provider dropped (the first film's own artwork then stands in). Returns
/// the source URL now on disk.
///
/// No `.await` inside a `match` guard here, on purpose. rustc 1.97
/// miscompiles `match array[i] { Some(x) if cond(x).await => .., Some(x)
/// => .. }` in an async fn: the element address is computed from an index
/// slot on the stack that is never written. In debug builds the first
/// image of the first collection read through a garbage index — on
/// Windows that crashed the whole server with STATUS_ACCESS_VIOLATION
/// right after a scan started (`artwork_sync_survives_a_dirty_stack`
/// guards against it coming back).
async fn sync_artwork(
    http: &reqwest::Client,
    config_dir: &Path,
    collection_id: Uuid,
    collection: &str,
    file_name: &str,
    stored_url: Option<String>,
    wanted_url: Option<&str>,
) -> Option<String> {
    let path = collection_image_path(config_dir, collection_id, file_name);
    let Some(url) = wanted_url else {
        if stored_url.is_some() {
            tracing::debug!(collection, image = file_name, "provider dropped this collection image");
        }
        let _ = tokio::fs::remove_file(&path).await;
        return None;
    };
    let unchanged = stored_url.as_deref() == Some(url);
    if unchanged && tokio::fs::try_exists(&path).await.unwrap_or(false) {
        return stored_url;
    }
    tracing::debug!(collection, image = file_name, url, path = %path.display(), "downloading collection artwork");
    match download_image(http, url, &path).await {
        Ok(()) => Some(url.to_owned()),
        Err(err) => {
            tracing::warn!(collection, %err, image = file_name, "failed to download collection artwork");
            // Whatever was on disk before is still there.
            stored_url
        }
    }
}

/// Deletes collections left without films (after a scan or refresh) and
/// their artwork, plus artwork folders no collection owns.
pub async fn prune_collections(pool: &PgPool, config_dir: &Path) {
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
            let kept = sync_artwork(&http, &dir, id, "Saga", "poster.jpg", Some(url.into()), Some(url)).await;
            assert_eq!(kept.as_deref(), Some(url));
            // A new URL that cannot be downloaded: the old one stays.
            let failed = sync_artwork(
                &http, &dir, id, "Saga", "poster.jpg", Some(url.into()), Some("http://127.0.0.1:9/new.jpg"),
            )
            .await;
            assert_eq!(failed.as_deref(), Some(url));
            // The provider dropped the image: file and URL go.
            let dropped = sync_artwork(&http, &dir, id, "Saga", "poster.jpg", Some(url.into()), None).await;
            assert_eq!(dropped, None);
        });
        assert!(!poster.exists());
        let _ = std::fs::remove_dir_all(&dir);
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
