//! Faza 5 completion's "Zadanie w tle" (background task): periodically
//! re-enriches every media item that isn't `metadata_locked`, using exactly
//! the same provider-priority / `forced_provider` / `locked_fields`-aware
//! machinery a manual scan's enrichment pass already uses (see
//! `metadata_enrichment::enrich_item_cached` and `scan_jobs::scan_library`, which this
//! module deliberately mirrors rather than reimplements its own copy of).
//! Every actual field-protection rule still lives exactly where
//! `scan_library` already enforces it — SQL-level, inside
//! `lisek_db::upsert_media_item`/`attach_provider_metadata` — so a locked
//! item or an individually locked field is exactly as safe from this job
//! as it is from an admin clicking "scan" by hand; this module only ever
//! decides *which items* and *how often*.
//!
//! Started once from `main.rs` via `tokio::spawn`, right after the app
//! begins listening — never blocks server startup, and never holds up a
//! request, since it runs on its own detached task for as long as the
//! process lives.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::admin::{apply_provider_order, item_type_for_library};
use crate::metadata_enrichment;
use crate::state::AppState;

/// How often a refresh pass runs. Not admin-configurable yet (see the
/// roadmap) — a fixed, conservative interval chosen so even a large
/// library doesn't hammer a free-tier provider API, while still catching
/// up items whose upstream metadata changed since they were last scanned
/// within the same day.
const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// This job's own stand-in for the scanning admin's UI language (see
/// `scan_jobs::scan_library`'s identical use of `auth.user.ui_language`) — a
/// background pass has no request/admin session to read one from, so it
/// always asks providers for English-prioritized images until a real
/// server-wide "metadata language" setting exists (see the roadmap).
const REFRESH_LANGUAGE: &str = "en";

/// Runs for as long as the process does: sleeps `REFRESH_INTERVAL`, then
/// runs one pass. Deliberately sleeps *before* the first pass rather than
/// running immediately on startup — a server that was just started (very
/// possibly mid-transfer, mid-migration, or about to be immediately
/// rescanned by an admin who just finished configuring a library) doesn't
/// need a redundant refresh pass racing that. A panic can't propagate out
/// of `run_once` and end this task silently, since every fallible step
/// inside it is already handled with `tracing::warn!` + `continue`/
/// `return` instead of `?`, matching every other long-running piece of
/// this codebase (see `metadata_enrichment::enrich_item_cached`, which this
/// module leans on directly).
pub async fn run(state: Arc<AppState>) {
    loop {
        tokio::time::sleep(REFRESH_INTERVAL).await;
        run_once(&state).await;
    }
}

async fn run_once(state: &Arc<AppState>) {
    let Some(pool) = state.pool() else {
        // Not `Ready` yet — a fresh install still sitting in the
        // First-Run Wizard, or a database that's temporarily unreachable.
        // Nothing to do this pass; the next one will simply check again.
        tracing::debug!("background metadata refresh: server not ready yet, skipping this pass");
        return;
    };

    let items = match lisek_db::list_unlocked_items_for_refresh(&pool).await {
        Ok(items) => items,
        Err(err) => {
            tracing::warn!(%err, "background metadata refresh: failed to list eligible items");
            return;
        }
    };
    if items.is_empty() {
        return;
    }

    let global_providers = match crate::provider_registry::enabled_provider_keys(&pool, &state.secrets).await {
        Ok(providers) => providers,
        Err(err) => {
            tracing::warn!(%err, "background metadata refresh: failed to list enabled providers");
            return;
        }
    };
    if global_providers.is_empty() {
        // Matches `scan_library`'s identical early-out: a real, common
        // first-run state (tmdb's row is enabled out of the box but still
        // needs an admin-supplied key), not an error worth escalating
        // every six hours.
        tracing::debug!(
            "background metadata refresh: no enabled/configured metadata provider — skipping this pass"
        );
        return;
    }

    let http = reqwest::Client::new();
    let config_dir = crate::state::config_dir();

    // Both caches are keyed per pass (not shared across passes) since a
    // library's `save_metadata_in_media_folder` setting or its provider
    // order can change between runs — six hours is cheap enough to just
    // re-read both once per pass rather than risk serving something stale
    // for an entire library's worth of items.
    let mut library_cache: HashMap<Uuid, Option<lisek_core::Library>> = HashMap::new();
    let mut order_cache: HashMap<(Uuid, &'static str), Vec<(String, String)>> = HashMap::new();

    // Items arrive grouped by series, so each show is looked up once.
    let mut cache = metadata_enrichment::EnrichCache::default();
    let mut items_matched = 0usize;
    let mut items_considered = 0usize;

    for (item_id, library_id, target, file_path, forced_provider) in items {
        let library = match library_cache.entry(library_id) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                let loaded = match lisek_db::get_library(&pool, library_id).await {
                    Ok(library) => library,
                    Err(err) => {
                        tracing::warn!(
                            %item_id, %library_id, %err,
                            "background metadata refresh: failed to load item's library"
                        );
                        None
                    }
                };
                entry.insert(loaded)
            }
        };
        // The library (and, via ON DELETE CASCADE, every one of its items)
        // was deleted since this pass started listing eligible items, or
        // its own lookup just failed above — either way, nothing sane to
        // enrich against.
        let Some(library) = library else {
            continue;
        };

        let item_type = item_type_for_library(&library.library_type);
        let providers = match order_cache.get(&(library_id, item_type)) {
            Some(order) => order.clone(),
            None => {
                let library_order =
                    lisek_db::get_library_provider_order(&pool, library_id, item_type)
                        .await
                        .unwrap_or_default();
                let ordered =
                    apply_provider_order(global_providers.clone(), library_order.as_deref());
                order_cache.insert((library_id, item_type), ordered.clone());
                ordered
            }
        };

        items_considered += 1;
        let outcome = metadata_enrichment::enrich_item_cached(
            &mut cache,
            &pool,
            &http,
            &config_dir,
            &providers,
            item_id,
            &target,
            &file_path,
            library.save_metadata_in_media_folder,
            REFRESH_LANGUAGE,
            forced_provider.as_deref(),
        )
        .await;
        if outcome.matched {
            items_matched += 1;
        }
    }

    tracing::info!(
        items_considered,
        items_matched,
        "background metadata refresh pass completed"
    );
}
