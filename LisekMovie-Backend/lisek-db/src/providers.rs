//! Metadata provider registry (`provider_config`) and per-library
//! provider order (`library_provider_config`).

use uuid::Uuid;

use lisek_metadata::{ProviderConfig, display_name_for};

use crate::{DbError, PgPool};

/// The raw shape every `provider_config` row comes back as. `settings` is
/// kept as a bare `serde_json::Value` here rather than a typed struct —
/// different providers can eventually want different settings shapes
/// (an API key today, maybe a base URL or region override later), and this
/// layer's job is just moving the JSONB in and out, not interpreting it.
type ProviderConfigRow = (String, bool, i32, serde_json::Value);

/// True when `settings` holds a non-empty `api_key` string — the only
/// thing any client of [`list_provider_configs`] gets to know about a
/// provider's settings; the raw value itself never leaves this function.
fn settings_has_api_key(settings: &serde_json::Value) -> bool {
    settings
        .get("api_key")
        .and_then(|value| value.as_str())
        .is_some_and(|key| !key.is_empty())
}

fn provider_config_from_row(row: ProviderConfigRow) -> ProviderConfig {
    let (provider_key, enabled, priority, settings) = row;
    ProviderConfig {
        display_name: display_name_for(&provider_key),
        is_configured: settings_has_api_key(&settings),
        provider_key,
        enabled,
        priority,
    }
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
pub async fn list_enabled_provider_keys(pool: &PgPool) -> Result<Vec<(String, String)>, DbError> {
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
            let api_key = settings.get("api_key")?.as_str()?;
            if api_key.is_empty() {
                return None;
            }
            Some((provider_key, api_key.to_string()))
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

/// Sets (or replaces) a provider's API key. Merges into the existing
/// `settings` JSONB via `||` rather than overwriting it outright, so a
/// future second settings field (a base URL, a region) set independently
/// isn't clobbered by this call.
pub async fn set_provider_api_key(
    pool: &PgPool,
    provider_key: &str,
    api_key: &str,
) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET settings = settings || jsonb_build_object('api_key', $2::text)
        WHERE provider_key = $1
        "#,
    )
    .bind(provider_key)
    .bind(api_key)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(DbError::ProviderNotFound);
    }
    Ok(())
}

/// Removes a provider's API key (if any) without touching any other
/// settings field, or its enabled/priority state.
pub async fn clear_provider_api_key(pool: &PgPool, provider_key: &str) -> Result<(), DbError> {
    let result = sqlx::query(
        r#"
        UPDATE provider_config
        SET settings = settings - 'api_key'
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
