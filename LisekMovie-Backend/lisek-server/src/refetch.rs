//! Faza 22: fetching an item's metadata again, now — after its ids were
//! changed in the metadata editor, or from the editor's "fetch again"
//! button. Runs the same enrichment as a scan (by the stored ids, with
//! the cascade and the language order) for a film or every episode of a
//! series, with each library's own plugin order.

use std::collections::HashMap;

use uuid::Uuid;

use crate::admin::{apply_provider_order, item_type_for_library};
use crate::metadata_enrichment::{self, EnrichCache};
use crate::state::SharedState;

/// What one refetch did.
#[derive(Debug, Default, Clone, Copy, serde::Serialize, utoipa::ToSchema)]
pub struct RefetchSummary {
    /// Items asked for (a series counts every episode).
    pub items: usize,
    /// Items a plugin answered for.
    pub matched: usize,
}

/// Enriches `items` again. Items with `metadata_locked` are left alone.
pub async fn refetch(
    state: &SharedState,
    pool: &lisek_db::PgPool,
    items: &[Uuid],
) -> Result<RefetchSummary, lisek_db::DbError> {
    let targets = lisek_db::identity::items_for_refresh(pool, items).await?;
    let mut summary = RefetchSummary::default();
    if targets.is_empty() {
        return Ok(summary);
    }
    let global = crate::provider_registry::enabled_provider_keys(pool, &state.secrets).await?;
    if global.is_empty() {
        tracing::info!("refetch: no enabled and configured plugin");
        return Ok(summary);
    }
    let language = lisek_db::profiles::default_language(pool)
        .await
        .unwrap_or_else(|_| "en".to_owned());
    let http = reqwest::Client::new();
    let config_dir = crate::state::config_dir();
    let mut cache = EnrichCache::default();
    let mut libraries: HashMap<Uuid, Option<lisek_core::Library>> = HashMap::new();
    for (item_id, library_id, target, file_path, forced) in targets {
        let library = match libraries.get(&library_id) {
            Some(library) => library.clone(),
            None => {
                let loaded = lisek_db::get_library(pool, library_id).await?;
                libraries.insert(library_id, loaded.clone());
                loaded
            }
        };
        let Some(library) = library else { continue };
        let item_type = item_type_for_library(&library.library_type);
        let order = lisek_db::get_library_provider_order(pool, library_id, item_type).await?;
        let providers = apply_provider_order(global.clone(), order.as_deref());
        summary.items += 1;
        let outcome = metadata_enrichment::enrich_item_cached(
            &mut cache,
            pool,
            &http,
            &config_dir,
            &providers,
            item_id,
            &target,
            &file_path,
            library.save_metadata_in_media_folder,
            &language,
            forced.as_deref(),
        )
        .await;
        summary.matched += usize::from(outcome.matched);
    }
    metadata_enrichment::prune_collections(pool, &config_dir).await;
    Ok(summary)
}
