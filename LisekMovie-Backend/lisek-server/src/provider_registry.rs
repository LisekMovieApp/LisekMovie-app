//! The one place a metadata provider extension gets wired into this binary.
//!
//! Per the documented boundary between the core and an extension: a new
//! `LisekMovie-plugin-*` extension is a normal Cargo path dependency (see
//! `Cargo.toml` — no dynamic `.dll`/`.so` loading, Rust has no safe way to
//! do that across separately compiled artifacts) plus exactly one match
//! arm here. Everything else — enabled/disabled, priority, the API key —
//! is read fresh from `provider_config` on every scan by
//! `scan_jobs::scan_library` (`lisek_db::list_enabled_provider_keys`), never
//! decided at startup, so flipping a provider on/off from the Admin Panel
//! is instant, no restart.

use lisek_plugin_api::MetadataProvider;

/// Builds the provider named `provider_key`, ready to call, from its
/// currently-configured `api_key`. `None` for a `provider_key` no
/// registered extension claims — `metadata_enrichment::try_provider` logs
/// that case and moves on to the next provider in the admin's priority
/// list, the same way a real "no match" from a known provider already
/// works.
pub fn build_provider(provider_key: &str, api_key: &str) -> Option<Box<dyn MetadataProvider>> {
    use lisek_plugin_anidb::{AnidbProvider, MIN_INTERVAL};
    use lisek_plugin_anilist::AnilistProvider;
    use lisek_plugin_imdb::ImdbProvider;
    use lisek_plugin_imvdb::ImvdbProvider;
    use lisek_plugin_omdb::OmdbProvider;
    use lisek_plugin_thetvdb::TvdbProvider;
    use lisek_plugin_tmdb::TmdbProvider;

    let url = base_url_override(provider_key);
    let provider: Box<dyn MetadataProvider> = match (provider_key, url) {
        ("tmdb", None) => Box::new(TmdbProvider::new(api_key)),
        ("tmdb", Some(url)) => Box::new(TmdbProvider::with_base_url(api_key, url)),
        ("tvdb", None) => Box::new(TvdbProvider::new(api_key)),
        ("tvdb", Some(url)) => Box::new(TvdbProvider::with_base_url(api_key, url)),
        ("imdb", None) => Box::new(ImdbProvider::new()),
        ("imdb", Some(url)) => Box::new(ImdbProvider::with_base_url(url)),
        ("omdb", None) => Box::new(OmdbProvider::new(api_key)),
        ("omdb", Some(url)) => Box::new(OmdbProvider::with_base_url(api_key, url)),
        ("anilist", None) => Box::new(AnilistProvider::new()),
        ("anilist", Some(url)) => Box::new(AnilistProvider::with_base_url(url)),
        ("anidb", None) => Box::new(AnidbProvider::new(api_key)),
        ("anidb", Some(url)) => Box::new(AnidbProvider::with_base_url(api_key, url, MIN_INTERVAL)),
        ("anisearch", _) => Box::new(lisek_plugin_anisearch::AnisearchProvider::new(api_key)),
        ("imvdb", None) => Box::new(ImvdbProvider::new(api_key)),
        ("imvdb", Some(url)) => Box::new(ImvdbProvider::with_base_url(api_key, url)),
        ("vgmdb", _) => Box::new(lisek_plugin_vgmdb::VgmdbProvider::new()),
        ("audiodb", _) => Box::new(lisek_plugin_audiodb::AudiodbProvider::new(api_key)),
        _ => return None,
    };
    Some(provider)
}

/// `LISEK_PROVIDER_URL_<KEY>` (e.g. `LISEK_PROVIDER_URL_IMDB`): another
/// base URL for one provider's API — a self-hosted mirror or proxy, or the
/// mock server of the integration suite.
fn base_url_override(provider_key: &str) -> Option<String> {
    std::env::var(format!("LISEK_PROVIDER_URL_{}", provider_key.to_ascii_uppercase()))
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
}

/// Enabled providers that have a key, in the admin's priority order, with
/// each key decrypted. A key that cannot be opened (the secret key changed
/// or was lost) is skipped with a warning; the admin re-enters it.
pub async fn enabled_provider_keys(
    pool: &lisek_db::PgPool,
    secrets: &lisek_auth::secrets::SecretBox,
) -> Result<Vec<(String, String)>, lisek_db::DbError> {
    use lisek_db::StoredApiKey;
    Ok(lisek_db::list_enabled_provider_keys(pool)
        .await?
        .into_iter()
        .filter_map(|(provider, stored)| match stored {
            StoredApiKey::Plain(key) => Some((provider, key)),
            StoredApiKey::Sealed(sealed) => match secrets.open(&sealed, &provider) {
                Ok(key) => Some((provider, key)),
                Err(err) => {
                    tracing::warn!(
                        provider,
                        %err,
                        "cannot decrypt this provider's API key (was LISEK_SECRET_KEY changed?) \
                         — enter it again under Admin Panel \u{2192} Plugins"
                    );
                    None
                }
            },
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_tmdb_provider() {
        let provider = build_provider("tmdb", "key").expect("tmdb should be registered");
        assert_eq!(provider.provider_key(), "tmdb");
    }

    #[test]
    fn builds_the_tvdb_provider() {
        let provider = build_provider("tvdb", "key").expect("tvdb should be registered");
        assert_eq!(provider.provider_key(), "tvdb");
    }

    #[test]
    fn every_listed_provider_is_registered_under_its_own_key() {
        for info in lisek_metadata::PROVIDERS {
            let provider = build_provider(info.key, "").unwrap_or_else(|| panic!("{} is not registered", info.key));
            assert_eq!(provider.provider_key(), info.key);
        }
    }

    #[test]
    fn returns_none_for_an_unregistered_provider_key() {
        assert!(build_provider("some-future-plugin", "key").is_none());
    }
}
