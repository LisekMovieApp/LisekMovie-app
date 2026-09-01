//! Shared contract (trait) for external metadata provider extensions.
//!
//! Faza 7 per the roadmap: this crate is what a `LisekMovie-plugin-*`
//! extension (TMDB, TVDB, and later others — see the roadmap's "resolved
//! decisions" section) implements to become a metadata source `lisek-
//! server` can call, and nothing more. Per the documented crate-boundary
//! rule, an extension depends on this crate and `lisek-core` only — never
//! on `lisek-db` or `lisek-server` directly — so this crate must never
//! grow a dependency on either of those either; the moment it needs one,
//! that's a sign a type belongs in `lisek-core` instead.
//!
//! Rust has no safe, production-ready equivalent of a managed runtime's
//! dynamic-assembly-loading facility: loading a `.dll`/`.so` at runtime has
//! no stable ABI across compiler versions or build flags, so it isn't a
//! sound way to load code compiled separately from the binary that loads
//! it. So "plugin" here means a normal Rust trait implemented by a normal
//! Cargo crate, compiled directly into the `lisek-server` binary as a path
//! dependency — not a `.dll`/`.so` loaded at runtime. What *does* stay
//! dynamic, and deliberately so, is which providers are enabled, their
//! priority, and their API keys: those live in `provider_config` and are
//! read fresh on every request (see `lisek-server`'s provider registry
//! module), so flipping a provider on/off from the Admin Panel never
//! requires a server restart.

use async_trait::async_trait;
use thiserror::Error;

/// What `lisek-server` is asking a provider to look up for a movie.
/// Deliberately not `lisek_db::EnrichTarget` — this crate cannot depend on
/// `lisek-db` (see this module's own doc comment) — and deliberately two
/// separate query types rather than one enum a provider has to match on:
/// a series-only provider (TVDB) simply never receives a [`MovieQuery`],
/// rather than being handed an enum variant it has to remember to reject.
#[derive(Debug, Clone, Copy)]
pub struct MovieQuery<'a> {
    pub title: &'a str,
    /// The year parsed from the filename, when the scanner found one —
    /// providers use this to disambiguate a remake/reboot, not as a strict
    /// filter (see `LisekMovie-plugin-tmdb`'s retry-without-year logic for
    /// why a strict filter alone isn't good enough).
    pub year: Option<i32>,
}

/// Same as [`MovieQuery`] for a TV series — no year, since the scanner's
/// series detection doesn't parse one out of the series-level folder name.
#[derive(Debug, Clone, Copy)]
pub struct SeriesQuery<'a> {
    pub title: &'a str,
}

/// One provider's match for an item: whatever fields it wants merged into
/// the item's `media_items.metadata` (namespaced under `"provider_data"`
/// by the caller — see `lisek_db::attach_provider_metadata`), plus ready-
/// to-download poster/backdrop URLs when it has them. A provider only ever
/// returns a URL here — `lisek-server`'s `metadata_enrichment` module is
/// what actually downloads and writes the image to disk (media folder or
/// local cache); no provider extension touches the filesystem itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderMatch {
    pub fields: serde_json::Value,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("request to the provider failed: {0}")]
    Request(String),
    #[error("the provider rejected the configured API key")]
    Unauthorized,
    #[error("the provider returned an unexpected response: {0}")]
    UnexpectedResponse(String),
}

/// Implemented by every metadata provider extension (`LisekMovie-plugin-*`)
/// — backs the admin-facing provider registry (enable/disable, priority,
/// API key), and `lisek-server`'s provider registry module is the one line
/// a new extension adds to become callable (no dynamic discovery — see
/// this crate's own doc comment for why).
///
/// A provider that only covers one item type (TVDB: series only) still
/// implements both methods; the one it doesn't support just always
/// returns `Ok(None)` — indistinguishable to the caller from "this
/// provider covers movies but didn't recognize this particular title",
/// which is exactly the point: `lisek-server` never needs to know which
/// providers cover which item types, it just asks and gets `None` either
/// way.
#[async_trait]
pub trait MetadataProvider: Send + Sync {
    /// This provider's stable key (`"tmdb"`, `"tvdb"`, ...) — must match
    /// the `provider_key` column values `lisek-db`'s migrations seed into
    /// `provider_config`, and the key the registry module maps to this
    /// implementation.
    fn provider_key(&self) -> &'static str;

    async fn find_movie(
        &self,
        query: &MovieQuery<'_>,
        primary_language: &str,
    ) -> Result<Option<ProviderMatch>, ProviderError>;

    async fn find_series(
        &self,
        query: &SeriesQuery<'_>,
        primary_language: &str,
    ) -> Result<Option<ProviderMatch>, ProviderError>;
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-plugin-api");
    }
}
