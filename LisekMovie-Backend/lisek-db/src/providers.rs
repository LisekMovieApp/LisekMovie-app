//! Metadata provider registry (`provider_config`) and per-library
//! provider order (`library_provider_config`).

use uuid::Uuid;

use lisek_metadata::{ProviderConfig, works_without_api_key};

use crate::{DbError, PgPool};

/// The raw shape every `provider_config` row comes back as. `settings` is
/// kept as a bare `serde_json::Value` here rather than a typed struct —
/// different providers can eventually want different settings shapes
/// (an API key today, maybe a base URL or region override later), and this
/// layer's job is just moving the JSONB in and out, not interpreting it.
type ProviderConfigRow = (String, bool, i32, serde_json::Value);

/// `settings` key of an API key encrypted by `lisek_auth::secrets`.
const SEALED_KEY: &str = "api_key_sealed";
/// `settings` key of a plaintext API key written before encryption existed;
/// [`seal_legacy_provider_keys`] converts it on startup.
const LEGACY_KEY: &str = "api_key";

/// True when `settings` holds a non-empty API key (sealed or legacy) — the
/// only thing any client of [`list_provider_configs`] gets to know about a
/// provider's settings; the raw value itself never leaves this module.
fn settings_has_api_key(settings: &serde_json::Value) -> bool {
    stored_api_key(settings).is_some()
}

/// A provider API key as stored. `lisek-db` never decrypts: opening a
/// sealed key needs the server's secret, which only `lisek-server` holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredApiKey {
    Sealed(String),
    /// Written before keys were encrypted; still usable until sealed.
    Plain(String),
}

fn stored_api_key(settings: &serde_json::Value) -> Option<StoredApiKey> {
    let field = |name: &str| {
        settings
            .get(name)
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    field(SEALED_KEY)
        .map(StoredApiKey::Sealed)
        .or_else(|| field(LEGACY_KEY).map(StoredApiKey::Plain))
}

fn provider_config_from_row(row: ProviderConfigRow) -> ProviderConfig {
    let (provider_key, enabled, priority, settings) = row;
    ProviderConfig::new(provider_key, enabled, priority, settings_has_api_key(&settings))
}

/// Every registered metadata provider (built-in or, later, an external
/// plugin's), ordered by admin-configured priority. Backs the Admin
/// Panel's plugins tab.
pub async fn list_provider_configs(pool: &PgPool) -> Result<Vec<ProviderConfig>, DbError> {
    let rows = sqlx::query_as::<_, ProviderConfigRow>(
        r#"
        SELECT provider_key, enabled, priority, settings
        FROM provider_config
        ORDER BY priority ASC, provider_key ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(provider_config_from_row).collect())
}

/// Every *enabled and configured* provider's raw API key, in admin-set
/// priority order — server-internal only. Unlike [`list_provider_configs`]
/// (which deliberately never exposes a stored key, only whether one
/// exists — see [`ProviderConfig`]'s own doc comment), this is what lets
/// `lisek-server` actually build a `TmdbClient`/`TvdbClient` to call out to
/// a provider on the admin's behalf; nothing here is ever serialized back
/// to an HTTP response.
pub async fn list_enabled_provider_keys(
    pool: &PgPool,
) -> Result<Vec<(String, StoredApiKey)>, DbError> {
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT provider_key, settings
        FROM provider_config
        WHERE enabled
        ORDER BY priority ASC, provider_key ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|(provider_key, settings)| {
            // Providers that need no key (IMDb, AniList...) run with an
            // empty one until the admin stores one.
            stored_api_key(&settings)
                .or_else(|| works_without_api_key(&provider_key).then(|| StoredApiKey::Plain(String::new())))
                .map(|key| (provider_key, key))
        })
        .collect())
}

/// Updates a provider's enabled state and display priority. Returns
/// [`DbError::ProviderNotFound`] if `provider_key` doesn't match a row —
/// callers use this to tell "updated" apart from "no such provider" without
/// a separate existence check.
pub async fn set_provider_enabled_and_priority(
    pool: &PgPool,
    provider_key: &str,
    enabled: bool,
    priority: i32,
) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET enabled = $2, priority = $3
        WHERE provider_key = $1
        "#,
    )
    .bind(provider_key)
    .bind(enabled)
    .bind(priority)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::ProviderNotFound);
    }
    Ok(())
}

/// Sets (or replaces) a provider's API key. `sealed_key` must already be
/// encrypted (`SecretBox::seal`, with the provider key as context); any
/// legacy plaintext key is dropped in the same statement. Merges into the
/// existing `settings` JSONB via `||` rather than overwriting it outright,
/// so a future second settings field (a base URL, a region) set
/// independently isn't clobbered by this call.
pub async fn set_provider_api_key(
    pool: &PgPool,
    provider_key: &str,
    sealed_key: &str,
) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET settings = (settings - 'api_key') || jsonb_build_object('api_key_sealed', $2::text)
        WHERE provider_key = $1
        "#,
    )
    .bind(provider_key)
    .bind(sealed_key)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::ProviderNotFound);
    }
    Ok(())
}

/// Encrypts every plaintext API key left from before keys were sealed.
/// `seal(provider_key, plaintext)` returns the sealed form. Idempotent;
/// returns how many keys were converted.
pub async fn seal_legacy_provider_keys(
    pool: &PgPool,
    seal: impl Fn(&str, &str) -> String,
) -> Result<u64, DbError> {
    let legacy: Vec<(String, String)> = sqlx::query_as(
        "SELECT provider_key, settings->>'api_key' FROM provider_config \
         WHERE settings->>'api_key' <> ''",
    )
    .fetch_all(pool)
    .await?;
    let mut sealed = 0;
    for (provider_key, plaintext) in legacy {
        set_provider_api_key(pool, &provider_key, &seal(&provider_key, &plaintext)).await?;
        sealed += 1;
    }
    // Empty legacy values carry nothing worth sealing.
    sqlx::query("UPDATE provider_config SET settings = settings - 'api_key' WHERE settings ? 'api_key'")
        .execute(pool)
        .await?;
    Ok(sealed)
}

/// Removes a provider's API key (if any) without touching any other
/// settings field, or its enabled/priority state.
pub async fn clear_provider_api_key(pool: &PgPool, provider_key: &str) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET settings = settings - 'api_key' - 'api_key_sealed'
        WHERE provider_key = $1
        "#,
    )
    .bind(provider_key)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::ProviderNotFound);
    }
    Ok(())
}

/// A library's saved custom provider order for one item type (`"movie"` or
/// `"episode"`) — `library_provider_config`'s own Faza 1 schema, unused
/// by any endpoint until the Admin Panel's drag-and-drop reordering UI.
/// `None` means no override has ever been saved for this (library,
/// item_type) pair, which the caller treats as "use the admin's global
/// priority order" (see `lisek-server`'s `provider_registry` doc comment
/// on where that global order itself comes from).
pub async fn get_library_provider_order(
    pool: &PgPool,
    library_id: Uuid,
    item_type: &str,
) -> Result<Option<Vec<String>>, DbError> {
    let row: Option<(Vec<String>,)> = sqlx::query_as(
        "SELECT provider_order FROM library_provider_config WHERE library_id = $1 AND item_type = $2",
    )
    .bind(library_id)
    .bind(item_type)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(order,)| order))
}

/// Saves a library's custom provider order for one item type — an upsert,
/// since the Admin Panel's drag-and-drop list always submits the whole
/// order in one request rather than one move at a time.
pub async fn set_library_provider_order(
    pool: &PgPool,
    library_id: Uuid,
    item_type: &str,
    order: &[String],
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO library_provider_config (library_id, item_type, provider_order)
        VALUES ($1, $2, $3)
        ON CONFLICT (library_id, item_type) DO UPDATE SET provider_order = EXCLUDED.provider_order
        "#,
    )
    .bind(library_id)
    .bind(item_type)
    .bind(order)
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_has_api_key_is_false_for_empty_or_missing() {
        assert!(!settings_has_api_key(&serde_json::json!({})));
        assert!(!settings_has_api_key(&serde_json::json!({ "api_key": "" })));
        assert!(!settings_has_api_key(&serde_json::json!({ "other": "x" })));
    }

    #[test]
    fn stored_api_key_prefers_the_sealed_value() {
        let settings = serde_json::json!({ "api_key": "plain", "api_key_sealed": "v1:abc" });
        assert_eq!(stored_api_key(&settings), Some(StoredApiKey::Sealed("v1:abc".into())));
        let legacy = serde_json::json!({ "api_key": "plain" });
        assert_eq!(stored_api_key(&legacy), Some(StoredApiKey::Plain("plain".into())));
        assert!(settings_has_api_key(&serde_json::json!({ "api_key_sealed": "v1:x" })));
    }

    #[test]
    fn settings_has_api_key_is_true_for_a_non_empty_key() {
        assert!(settings_has_api_key(
            &serde_json::json!({ "api_key": "abc123" })
        ));
    }

    #[test]
    fn provider_config_from_row_resolves_display_name_and_configured_state() {
        let config = provider_config_from_row((
            "tmdb".to_string(),
            true,
            0,
            serde_json::json!({ "api_key": "secret" }),
        ));
        assert_eq!(config.provider_key, "tmdb");
        assert_eq!(config.display_name, "TMDB");
        assert!(config.enabled);
        assert!(config.is_configured);
    }
}
