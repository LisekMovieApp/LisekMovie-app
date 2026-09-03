//! Metadata provider registry: ordering, overrides, locks; and the
//! generic (provider-agnostic) image cache every provider extension's
//! download goes through.
//!
//! Faza 5 added the admin-facing provider registry (`ProviderConfig`,
//! `BUILTIN_PROVIDERS`) backing the Admin Panel's plugins tab. The actual
//! TMDB/TVDB API clients that used to live here as `tmdb`/`tvdb` modules
//! moved out in Faza 7 into their own compile-time extension crates —
//! `LisekMovie-plugin-tmdb` and `LisekMovie-plugin-thetvdb`, both
//! implementing `lisek_plugin_api::MetadataProvider` — since a specific
//! provider's HTTP client is exactly the kind of "external, swappable data
//! source" the documented rdzeń/rozszerzenie boundary draws around an
//! extension, while [`image_cache`] (downloading and writing whatever
//! image URL a provider hands back) is generic infrastructure every
//! provider extension shares, so it stays here.

pub mod image_cache;

use serde::Serialize;

/// The metadata providers built directly into the core app, as opposed to a
/// future external plugin (Faza 7's `lisek-plugin-api` + a separate
/// `LisekMovie-plugin-*` repo). `(provider_key, display_name)` — the key is
/// what's stored in `provider_config.provider_key`; the display name is
/// only ever shown in the Admin Panel, never persisted.
pub const BUILTIN_PROVIDERS: &[(&str, &str)] = &[("tmdb", "TMDB"), ("tvdb", "TheTVDB")];

/// Looks up a provider's human-readable name, falling back to its raw key
/// for anything not in [`BUILTIN_PROVIDERS`] (a future external plugin, or
/// simply a key the app doesn't recognize yet).
pub fn display_name_for(provider_key: &str) -> String {
    BUILTIN_PROVIDERS
        .iter()
        .find(|(key, _)| *key == provider_key)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| provider_key.to_string())
}

/// A single metadata provider's admin-configurable state, as shown on the
/// Admin Panel's plugins tab: enabled/disabled, display priority, and
/// whether it currently has usable settings — without ever exposing the
/// settings themselves. `provider_config.settings` (JSONB) can hold a raw
/// API key; that value has no field on this type at all, by design, so
/// there's no accidental serialization path that could leak it back to a
/// client that only asked "is TMDB configured?". Setting or clearing the
/// key goes through its own dedicated write-only endpoint instead of a
/// general "update settings" one that could double as a read.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProviderConfig {
    pub provider_key: String,
    pub display_name: String,
    pub enabled: bool,
    pub priority: i32,
    pub is_configured: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-metadata");
    }

    #[test]
    fn display_name_resolves_known_builtin_providers() {
        assert_eq!(display_name_for("tmdb"), "TMDB");
        assert_eq!(display_name_for("tvdb"), "TheTVDB");
    }

    #[test]
    fn display_name_falls_back_to_the_raw_key_for_an_unknown_provider() {
        assert_eq!(display_name_for("some-future-plugin"), "some-future-plugin");
    }

    #[test]
    fn provider_config_never_serializes_a_settings_or_api_key_field() {
        let config = ProviderConfig {
            provider_key: "tmdb".to_string(),
            display_name: "TMDB".to_string(),
            enabled: true,
            priority: 0,
            is_configured: true,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("api_key"));
        assert!(!json.contains("settings"));
    }
}
