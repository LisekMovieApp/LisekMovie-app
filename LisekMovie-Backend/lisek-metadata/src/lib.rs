//! Metadata provider registry: ordering, overrides, locks; and the
//! generic (provider-agnostic) image cache every provider extension's
//! download goes through.
//!
//! Faza 5 added the admin-facing provider registry (`ProviderConfig`,
//! [`PROVIDERS`]) backing the Admin Panel's plugins tab. The actual
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
pub mod layout;

use serde::Serialize;

/// Whether a provider needs an API key (or a registered client name) in
/// Admin → Plugins before it can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyNeed {
    Required,
    /// Works without one; a key unlocks more (higher limits, more calls).
    Optional,
    None,
}

impl ApiKeyNeed {
    pub fn as_str(self) -> &'static str {
        match self {
            ApiKeyNeed::Required => "required",
            ApiKeyNeed::Optional => "optional",
            ApiKeyNeed::None => "none",
        }
    }
}

/// What a provider has data for. Shown in Admin → Plugins so a library's
/// provider order can be set per content (an anime library puts AniList or
/// aniDB first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCoverage {
    Movies,
    Series,
    Anime,
    MusicVideos,
    Music,
}

impl ProviderCoverage {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderCoverage::Movies => "movies",
            ProviderCoverage::Series => "series",
            ProviderCoverage::Anime => "anime",
            ProviderCoverage::MusicVideos => "music_videos",
            ProviderCoverage::Music => "music",
        }
    }
}

/// How complete a provider's extension is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStage {
    /// Search, lookup, artwork, people/seasons where the source has them.
    Full,
    /// Search and lookup with the main fields; parts are still missing.
    Basic,
    /// Registered, answers nothing yet (endpoints to be implemented).
    Skeleton,
}

impl ProviderStage {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderStage::Full => "full",
            ProviderStage::Basic => "basic",
            ProviderStage::Skeleton => "skeleton",
        }
    }
}

/// One metadata provider extension (`LisekMovie-plugin-*`) the server is
/// built with. `key` is `provider_config.provider_key` and the key
/// `lisek-server`'s provider registry builds the extension for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderInfo {
    pub key: &'static str,
    pub display_name: &'static str,
    pub api_key: ApiKeyNeed,
    pub covers: &'static [ProviderCoverage],
    pub stage: ProviderStage,
    /// Where an administrator gets a key or reads about the source.
    pub website: &'static str,
}

use ProviderCoverage::{Anime, Movies, Music, MusicVideos, Series};

/// Every provider extension, in the default priority order of
/// `provider_config` (migrations seed one row per entry).
pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        key: "tmdb",
        display_name: "TMDB",
        api_key: ApiKeyNeed::Required,
        covers: &[Movies, Series, Anime],
        stage: ProviderStage::Full,
        website: "https://www.themoviedb.org/settings/api",
    },
    ProviderInfo {
        key: "tvdb",
        display_name: "TheTVDB",
        api_key: ApiKeyNeed::Required,
        covers: &[Series, Anime],
        stage: ProviderStage::Basic,
        website: "https://thetvdb.com/api-information",
    },
    ProviderInfo {
        key: "imdb",
        display_name: "IMDb",
        api_key: ApiKeyNeed::None,
        covers: &[Movies, Series],
        stage: ProviderStage::Basic,
        website: "https://www.imdb.com",
    },
    ProviderInfo {
        key: "omdb",
        display_name: "OMDb",
        api_key: ApiKeyNeed::Required,
        covers: &[Movies, Series],
        stage: ProviderStage::Basic,
        website: "https://www.omdbapi.com/apikey.aspx",
    },
    ProviderInfo {
        key: "anilist",
        display_name: "AniList",
        api_key: ApiKeyNeed::None,
        covers: &[Anime],
        stage: ProviderStage::Basic,
        website: "https://anilist.co",
    },
    ProviderInfo {
        key: "anidb",
        display_name: "aniDB",
        api_key: ApiKeyNeed::Required,
        covers: &[Anime],
        stage: ProviderStage::Basic,
        website: "https://anidb.net/software/add",
    },
    ProviderInfo {
        key: "anisearch",
        display_name: "aniSearch",
        api_key: ApiKeyNeed::Optional,
        covers: &[Anime],
        stage: ProviderStage::Skeleton,
        website: "https://www.anisearch.com",
    },
    ProviderInfo {
        key: "imvdb",
        display_name: "IMVDb",
        api_key: ApiKeyNeed::Required,
        covers: &[MusicVideos],
        stage: ProviderStage::Basic,
        website: "https://imvdb.com/developers",
    },
    ProviderInfo {
        key: "vgmdb",
        display_name: "VGMdb",
        api_key: ApiKeyNeed::None,
        covers: &[Music],
        stage: ProviderStage::Skeleton,
        website: "https://vgmdb.net",
    },
    ProviderInfo {
        key: "audiodb",
        display_name: "TheAudioDB",
        api_key: ApiKeyNeed::Optional,
        covers: &[Music],
        stage: ProviderStage::Skeleton,
        website: "https://www.theaudiodb.com/api_guide.php",
    },
];

/// The extension registered under `provider_key`.
pub fn provider_info(provider_key: &str) -> Option<&'static ProviderInfo> {
    PROVIDERS.iter().find(|p| p.key == provider_key)
}

/// A provider's human-readable name, falling back to its raw key for
/// anything not in [`PROVIDERS`] (a row of an extension this build does not
/// have).
pub fn display_name_for(provider_key: &str) -> String {
    provider_info(provider_key)
        .map(|p| p.display_name.to_string())
        .unwrap_or_else(|| provider_key.to_string())
}

/// Whether `provider_key` can be called without a stored key. Unknown
/// providers need one.
pub fn works_without_api_key(provider_key: &str) -> bool {
    provider_info(provider_key).is_some_and(|p| p.api_key != ApiKeyNeed::Required)
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
    /// A key is stored, or the provider works without one.
    pub is_configured: bool,
    /// A key is stored (never the key itself).
    pub key_stored: bool,
    pub key_requirement: ApiKeyNeed,
    pub covers: Vec<ProviderCoverage>,
    pub stage: ProviderStage,
    pub website: Option<String>,
}

impl ProviderConfig {
    /// The admin view of a `provider_config` row: its state plus what the
    /// extension registered under the key says about itself.
    pub fn new(provider_key: String, enabled: bool, priority: i32, key_stored: bool) -> Self {
        let info = provider_info(&provider_key);
        let key_requirement = info.map_or(ApiKeyNeed::Required, |p| p.api_key);
        Self {
            display_name: display_name_for(&provider_key),
            is_configured: key_stored || key_requirement != ApiKeyNeed::Required,
            key_stored,
            key_requirement,
            covers: info.map(|p| p.covers.to_vec()).unwrap_or_default(),
            stage: info.map_or(ProviderStage::Skeleton, |p| p.stage),
            website: info.map(|p| p.website.to_string()),
            provider_key,
            enabled,
            priority,
        }
    }
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
    fn keyless_providers_count_as_configured() {
        let imdb = ProviderConfig::new("imdb".into(), true, 2, false);
        assert!(imdb.is_configured && !imdb.key_stored);
        assert_eq!(imdb.key_requirement, ApiKeyNeed::None);
        let omdb = ProviderConfig::new("omdb".into(), true, 3, false);
        assert!(!omdb.is_configured);
        let unknown = ProviderConfig::new("future".into(), true, 9, false);
        assert!(!unknown.is_configured);
        assert!(unknown.covers.is_empty());
        assert!(works_without_api_key("anilist"));
        assert!(!works_without_api_key("tmdb"));
        assert!(!works_without_api_key("future"));
        // Keys are unique.
        for (i, p) in PROVIDERS.iter().enumerate() {
            assert!(PROVIDERS[i + 1..].iter().all(|q| q.key != p.key), "{}", p.key);
        }
    }

    #[test]
    fn display_name_falls_back_to_the_raw_key_for_an_unknown_provider() {
        assert_eq!(display_name_for("some-future-plugin"), "some-future-plugin");
    }

    #[test]
    fn provider_config_never_serializes_a_settings_or_api_key_field() {
        let config = ProviderConfig::new("tmdb".to_string(), true, 0, true);
        let json = serde_json::to_value(&config).unwrap();
        // Field names only: values such as the website link may contain
        // these words.
        let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
        assert!(keys.iter().all(|k| !k.contains("api_key") && !k.contains("settings")), "{keys:?}");
    }
}
