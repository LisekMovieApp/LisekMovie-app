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
    match provider_key {
        "tmdb" => Some(Box::new(lisek_plugin_tmdb::TmdbProvider::new(api_key))),
        "tvdb" => Some(Box::new(lisek_plugin_thetvdb::TvdbProvider::new(api_key))),
        _ => None,
    }
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
    fn returns_none_for_an_unregistered_provider_key() {
        assert!(build_provider("some-future-plugin", "key").is_none());
    }
}
