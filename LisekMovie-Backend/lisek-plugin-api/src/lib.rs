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
/// to-download image URLs when it has them. A provider only ever
/// returns a URL here — `lisek-server`'s `metadata_enrichment` module is
/// what actually downloads and writes the image to disk (media folder or
/// local cache); no provider extension touches the filesystem itself.
///
/// `logo_url`/`banner_url` were added alongside `poster_url`/`backdrop_url`
/// once the admin asked for the fuller asset set a Hero Banner wants (a
/// transparent title logo instead of plain HTML text). Neither is
/// guaranteed populated by every provider: TMDB's `/images` endpoint has a
/// real `logos` array (see `LisekMovie-plugin-tmdb`) but no equivalent
/// "banner" artwork type at all, so `banner_url` is always `None` from
/// TMDB — there is no TMDB asset to put there, not a bug. TVDB v4 *does*
/// have wide-banner artwork, but its type ids are returned dynamically by
/// `GET /artwork/types` rather than being fixed, documented constants (the
/// way TMDB's genre ids are) — wiring that up needs its own verified
/// implementation, not a guessed type id, so it's left `None` here too for
/// now rather than risk shipping a silently-wrong artwork lookup.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProviderMatch {
    pub fields: serde_json::Value,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
    pub logo_url: Option<String>,
    pub banner_url: Option<String>,
    /// Cast and crew (Faza 8). `None`: the provider has no credits or the
    /// lookup failed — the item keeps the people it has. `Some(empty)`:
    /// the provider says nobody is credited.
    pub people: Option<Vec<ProviderPerson>>,
    /// A series' seasons (Faza 10): names, synopses and posters. Empty for
    /// movies and providers without season data.
    pub seasons: Vec<ProviderSeason>,
}

/// One season of a series match.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderSeason {
    /// 0 for specials.
    pub number: i32,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub episode_count: Option<i32>,
    pub air_year: Option<i32>,
}

/// One episode of a season ([`MetadataProvider::season_episodes`]).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProviderEpisode {
    pub season: i32,
    pub episode: i32,
    pub title: Option<String>,
    pub overview: Option<String>,
    pub runtime_minutes: Option<u32>,
    /// User score 0-10, only with votes.
    pub rating: Option<f64>,
    /// A 16:9 still from the episode.
    pub still_url: Option<String>,
    /// `YYYY-MM-DD`.
    pub air_date: Option<String>,
}

/// What a credited person did on a title. Stored as the variant name
/// (`media_people.role`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PersonRole {
    Actor,
    Director,
    Writer,
}

impl PersonRole {
    pub fn as_str(self) -> &'static str {
        match self {
            PersonRole::Actor => "Actor",
            PersonRole::Director => "Director",
            PersonRole::Writer => "Writer",
        }
    }
}

/// One credit of a [`ProviderMatch`]: a person and their part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPerson {
    /// The provider's own person id (TMDB: a number).
    pub provider_id: String,
    pub name: String,
    pub role: PersonRole,
    /// The character an actor plays.
    pub character: Option<String>,
    /// Billing order within the role, 0 first.
    pub order: i32,
    /// A profile photo, when the provider has one.
    pub profile_url: Option<String>,
}

/// A person's own page ([`MetadataProvider::person`]). Dates are
/// `YYYY-MM-DD`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderPersonDetails {
    pub name: String,
    pub biography: String,
    pub birth_date: Option<String>,
    pub death_date: Option<String>,
    pub birth_place: Option<String>,
    pub imdb_id: Option<String>,
    pub profile_url: Option<String>,
}

/// What kind of item a manual search or lookup ("Fix match") is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Movie,
    Series,
}

/// One search result offered to an administrator fixing a wrong match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCandidate {
    /// The provider's own id, as [`MetadataProvider::lookup`] takes it.
    pub provider_id: String,
    pub title: String,
    pub year: Option<i32>,
    pub overview: String,
    /// A small poster to recognize the title by, when the provider has one.
    pub poster_url: Option<String>,
}

impl ProviderCandidate {
    /// The candidate a [`MetadataProvider::lookup`] result stands for:
    /// `fields.title`, `fields.year` and `fields.overview` of the match, its
    /// poster, and `provider_id` as given. `None` when the match has no
    /// title (nothing an administrator could recognize).
    pub fn from_match(provider_id: &str, matched: &ProviderMatch) -> Option<Self> {
        let text = |key: &str| {
            matched
                .fields
                .get(key)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string()
        };
        let title = text("title");
        if title.is_empty() {
            return None;
        }
        Some(Self {
            provider_id: provider_id.to_string(),
            title,
            year: matched
                .fields
                .get("year")
                .and_then(serde_json::Value::as_i64)
                .and_then(|y| i32::try_from(y).ok()),
            overview: text("overview"),
            poster_url: matched.poster_url.clone(),
        })
    }
}

/// An id typed into "Fix match" instead of a title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectId {
    /// An IMDb title id (`tt0425622`). Several providers can resolve it:
    /// IMDb and OMDb directly, TMDB and TheTVDB through their
    /// external-id lookups.
    Imdb(String),
    /// A plain number: the id a provider uses itself (TMDB `603`,
    /// TheTVDB `60620`, AniList `21`, aniDB `69`...).
    Numeric(String),
}

/// What [`parse_id_query`] recognized in a Fix match query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdQuery {
    pub id: DirectId,
    /// The provider the id belongs to, when the input says so (`tmdb:603`,
    /// a themoviedb.org link). `None`: every provider may try it.
    pub provider: Option<&'static str>,
    /// A bare number without a prefix or link: it can just as well be a
    /// title ("1917", "300"), so callers search the text too.
    pub ambiguous: bool,
}

/// Prefixes (`tmdb:603`) and the provider key each one names. `imdb:`
/// names the id type, not the IMDb provider: every provider that resolves
/// IMDb ids is asked.
const ID_PREFIXES: &[(&str, Option<&str>)] = &[
    ("imdb", None),
    ("omdb", Some("omdb")),
    ("tmdb", Some("tmdb")),
    ("tvdb", Some("tvdb")),
    ("thetvdb", Some("tvdb")),
    ("anidb", Some("anidb")),
    ("anilist", Some("anilist")),
    ("anisearch", Some("anisearch")),
    ("imvdb", Some("imvdb")),
    ("vgmdb", Some("vgmdb")),
    ("audiodb", Some("audiodb")),
];

/// Links of provider pages: `(host fragment, path prefix, provider key)`.
/// The number right after the prefix is the id.
const ID_LINKS: &[(&str, &str, &str)] = &[
    ("themoviedb.org", "/movie/", "tmdb"),
    ("themoviedb.org", "/tv/", "tmdb"),
    ("thetvdb.com", "/dereferrer/series/", "tvdb"),
    ("anilist.co", "/anime/", "anilist"),
    ("anidb.net", "/anime/", "anidb"),
    ("anidb.net", "/a", "anidb"),
    ("anisearch.com", "/anime/", "anisearch"),
    ("anisearch.de", "/anime/", "anisearch"),
    ("vgmdb.net", "/album/", "vgmdb"),
];

/// `tt` followed by 7 to 10 digits.
fn imdb_id(value: &str) -> Option<String> {
    let digits = value.strip_prefix("tt")?;
    ((7..=10).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.to_string())
}

/// 1 to 10 digits.
fn numeric_id(value: &str) -> Option<String> {
    ((1..=10).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.to_string())
}

fn leading_digits(value: &str) -> &str {
    let end = value.find(|c: char| !c.is_ascii_digit()).unwrap_or(value.len());
    &value[..end]
}

/// Recognizes an id in a Fix match query: an IMDb id (`tt0425622`), a
/// plain number (`60620`), a prefixed id (`tvdb:60620`, `anilist:21`) or a
/// provider page link (`https://www.imdb.com/title/tt0425622/`,
/// `https://www.themoviedb.org/movie/603-the-matrix`). `None`: a title.
pub fn parse_id_query(input: &str) -> Option<IdQuery> {
    let input = input.trim();
    if input.is_empty() || input.contains(char::is_whitespace) {
        return None;
    }
    let lower = input.to_ascii_lowercase();

    if lower.contains("://") || lower.starts_with("www.") {
        let rest = lower.split_once("://").map_or(lower.as_str(), |(_, r)| r);
        let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let path = path.split(['?', '#']).next().unwrap_or_default();
        if host.ends_with("imdb.com") {
            let id = path.strip_prefix("/title/")?.split('/').next()?;
            return imdb_id(id).map(|id| IdQuery { id: DirectId::Imdb(id), provider: None, ambiguous: false });
        }
        if host.ends_with("thetvdb.com") {
            // Old-style `?id=60620` links.
            if let Some(id) = rest.split_once("id=").map(|(_, v)| leading_digits(v)).and_then(numeric_id) {
                return Some(IdQuery { id: DirectId::Numeric(id), provider: Some("tvdb"), ambiguous: false });
            }
        }
        return ID_LINKS.iter().find_map(|(link_host, prefix, provider)| {
            if !host.ends_with(link_host) {
                return None;
            }
            let id = numeric_id(leading_digits(path.strip_prefix(prefix)?))?;
            Some(IdQuery { id: DirectId::Numeric(id), provider: Some(provider), ambiguous: false })
        });
    }

    if let Some((prefix, value)) = lower.split_once(':') {
        let (_, provider) = ID_PREFIXES.iter().find(|(p, _)| *p == prefix)?;
        let id = match provider {
            None => DirectId::Imdb(imdb_id(value)?),
            Some(_) => imdb_id(value)
                .map(DirectId::Imdb)
                .or_else(|| numeric_id(value).map(DirectId::Numeric))?,
        };
        return Some(IdQuery { id, provider: *provider, ambiguous: false });
    }

    if let Some(id) = imdb_id(&lower) {
        return Some(IdQuery { id: DirectId::Imdb(id), provider: None, ambiguous: false });
    }
    numeric_id(&lower).map(|id| IdQuery { id: DirectId::Numeric(id), provider: None, ambiguous: true })
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

    /// Up to ~10 candidates for a title typed by an administrator ("Fix
    /// match"), best first. `title` is searched as given (no automatic
    /// shortening); `year` narrows when set. A provider that does not cover
    /// `kind` returns an empty list.
    async fn search(
        &self,
        _kind: MediaKind,
        _title: &str,
        _year: Option<i32>,
        _primary_language: &str,
    ) -> Result<Vec<ProviderCandidate>, ProviderError> {
        Ok(Vec::new())
    }

    /// The full match for one of this provider's ids (a candidate the
    /// administrator picked). Same shape as `find_movie`/`find_series`,
    /// plus `fields.title` and `fields.year` with the provider's own title
    /// and release year. `Ok(None)`: no such id.
    async fn lookup(
        &self,
        _kind: MediaKind,
        _provider_id: &str,
        _primary_language: &str,
    ) -> Result<Option<ProviderMatch>, ProviderError> {
        Ok(None)
    }

    /// The candidate for an id typed into Fix match ([`parse_id_query`]).
    /// The default resolves the provider's own numeric ids through
    /// [`Self::lookup`]; a provider that can map IMDb ids (TMDB's `/find`,
    /// TheTVDB's remote ids, OMDb) overrides this. The candidate's
    /// `provider_id` is always the provider's own id, as
    /// [`Self::lookup`] takes it. `Ok(None)`: not this provider's id, or no
    /// such title.
    async fn lookup_by_id(
        &self,
        kind: MediaKind,
        id: &DirectId,
        primary_language: &str,
    ) -> Result<Option<ProviderCandidate>, ProviderError> {
        match id {
            DirectId::Numeric(own) => Ok(self
                .lookup(kind, own, primary_language)
                .await?
                .and_then(|matched| ProviderCandidate::from_match(own, &matched))),
            DirectId::Imdb(_) => Ok(None),
        }
    }

    /// Every episode of one season of a series this provider matched
    /// (`series_id`: the provider's own id, as in `fields.tmdb_id`). Empty
    /// for a provider without episode data.
    async fn season_episodes(
        &self,
        _series_id: &str,
        _season: i32,
        _primary_language: &str,
    ) -> Result<Vec<ProviderEpisode>, ProviderError> {
        Ok(Vec::new())
    }

    /// One person's biography and dates, for an id this provider returned
    /// in [`ProviderMatch::people`]. `Ok(None)`: no such person, or the
    /// provider has no people at all.
    async fn person(
        &self,
        _provider_id: &str,
        _primary_language: &str,
    ) -> Result<Option<ProviderPersonDetails>, ProviderError> {
        Ok(None)
    }
}

/// Lowercases `title` and turns punctuation a provider's search backend
/// generally treats as noise — colons, commas, semicolons, dashes, periods,
/// and similar marks — into a plain word boundary, collapsing whitespace
/// runs down to single spaces. Apostrophes are dropped outright rather than
/// turned into a space (`"Ocean's Eleven"` → `"oceans eleven"`, not `"ocean
/// s eleven"` — the latter is a worse query, not a cleaner one).
///
/// Safe to apply unconditionally, to every query this crate's provider
/// extensions send: a title that was already plain text comes back
/// unchanged in substance (every provider's search is already
/// case-insensitive), while a title still carrying scene-release
/// punctuation or subtitle separators — `"Movie: The Subtitle"`,
/// `"Asterix.I.Obelix..."`-style dotted names that slipped past
/// `lisek-library`'s own cleanup — comes back normalized into the plain,
/// spaced-out form providers actually index well.
pub fn normalize_query_title(title: &str) -> String {
    let lowered = title.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    for ch in lowered.chars() {
        match ch {
            '\'' | '\u{2019}' | '`' => {}
            c if c.is_alphanumeric() || c.is_whitespace() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Folds a handful of Latin-alphabet diacritics — the Polish and Czech
/// letters this app's own shipped locales (PL/EN/CS) actually use — down to
/// their plain ASCII base letter. Not a general Unicode normalizer (no
/// `unicode-normalization` dependency pulled in for it): a small, explicit
/// table matches this project's existing preference for plain, auditable
/// Rust over a general-purpose crate for a narrow, known need — see
/// `lisek-library`'s own hand-rolled junk-token lists for the same idea
/// applied to filename parsing.
fn strip_diacritics_char(ch: char) -> char {
    match ch {
        'ą' => 'a',
        'ć' => 'c',
        'ę' => 'e',
        'ł' => 'l',
        'ń' => 'n',
        'ó' => 'o',
        'ś' => 's',
        'ź' | 'ż' => 'z',
        'á' => 'a',
        'č' => 'c',
        'ď' => 'd',
        'é' | 'ě' => 'e',
        'í' => 'i',
        'ň' => 'n',
        'ř' => 'r',
        'š' => 's',
        'ť' => 't',
        'ú' | 'ů' => 'u',
        'ý' => 'y',
        // Czech 'ž' (U+017E, LATIN SMALL LETTER Z WITH CARON) is a
        // different character from Polish 'ż' (U+017C, ...WITH DOT ABOVE)
        // above despite looking near-identical — easy to drop by accident
        // in a hand-rolled table exactly like this one; the test this fixed
        // is what caught it.
        'ž' => 'z',
        other => other,
    }
}

/// Applies [`strip_diacritics_char`] to every character of `title` — call
/// this on an already-[`normalize_query_title`]'d (lowercased) string, not
/// raw input; the table above only covers lowercase letters.
pub fn strip_diacritics(title: &str) -> String {
    title.chars().map(strip_diacritics_char).collect()
}

/// The first `n` whitespace-separated words of `title`, rejoined with
/// single spaces. Used to build a deliberately loose last-resort search
/// query — see [`query_candidates`] — for a title a provider's search
/// rejected in full; fewer words than `n` in `title` just returns what's
/// there instead of padding or erroring.
pub fn first_words(title: &str, n: usize) -> String {
    title
        .split_whitespace()
        .take(n)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Builds an ordered list of increasingly loose search-query candidates out
/// of a scanned title, for a provider extension to try in order and stop at
/// the first real match: the [`normalize_query_title`]'d title first, then
/// — only when it actually differs — a [`strip_diacritics`]'d variant, then
/// — only when *that* differs from every earlier entry and isn't empty —
/// just its [`first_words`]`(_, 2)`. A title with no diacritics and only
/// one or two words to begin with comes back as a single-entry list, not
/// three copies of the same string; every provider extension using this is
/// still responsible for its own per-candidate retry policy (e.g. TMDB's
/// own year → no-year retry), this only ever varies the *title* text.
pub fn query_candidates(title: &str) -> Vec<String> {
    let normalized = normalize_query_title(title);
    let mut candidates = vec![normalized.clone()];

    let diacritics_stripped = strip_diacritics(&normalized);
    if diacritics_stripped != normalized {
        candidates.push(diacritics_stripped.clone());
    }

    let fallback = first_words(&diacritics_stripped, 2);
    if !fallback.is_empty() && !candidates.contains(&fallback) {
        candidates.push(fallback);
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(input: &str) -> Option<(DirectId, Option<&'static str>, bool)> {
        parse_id_query(input).map(|q| (q.id, q.provider, q.ambiguous))
    }

    #[test]
    fn recognizes_imdb_ids_numbers_prefixes_and_links() {
        let imdb = |v: &str| DirectId::Imdb(v.to_string());
        let num = |v: &str| DirectId::Numeric(v.to_string());
        assert_eq!(id("tt0425622"), Some((imdb("tt0425622"), None, false)));
        assert_eq!(id(" TT0425622 "), Some((imdb("tt0425622"), None, false)));
        assert_eq!(id("60620"), Some((num("60620"), None, true)));
        assert_eq!(id("tvdb:60620"), Some((num("60620"), Some("tvdb"), false)));
        assert_eq!(id("TheTVDB:60620"), Some((num("60620"), Some("tvdb"), false)));
        assert_eq!(id("imdb:tt0133093"), Some((imdb("tt0133093"), None, false)));
        assert_eq!(id("tmdb:tt0133093"), Some((imdb("tt0133093"), Some("tmdb"), false)));
        assert_eq!(id("anilist:21"), Some((num("21"), Some("anilist"), false)));
        assert_eq!(id("omdb:tt0133093"), Some((imdb("tt0133093"), Some("omdb"), false)));
        assert_eq!(
            id("https://www.imdb.com/title/tt0425622/?ref_=nv_sr_1"),
            Some((imdb("tt0425622"), None, false))
        );
        assert_eq!(
            id("https://www.themoviedb.org/movie/603-the-matrix"),
            Some((num("603"), Some("tmdb"), false))
        );
        assert_eq!(id("https://www.themoviedb.org/tv/1396"), Some((num("1396"), Some("tmdb"), false)));
        assert_eq!(
            id("https://thetvdb.com/?tab=series&id=81189"),
            Some((num("81189"), Some("tvdb"), false))
        );
        assert_eq!(id("https://anilist.co/anime/21/ONE-PIECE/"), Some((num("21"), Some("anilist"), false)));
        assert_eq!(id("https://anidb.net/anime/69"), Some((num("69"), Some("anidb"), false)));
        assert_eq!(id("https://anidb.net/a69"), Some((num("69"), Some("anidb"), false)));
        assert_eq!(id("https://www.anisearch.com/anime/2227,one-piece"), Some((num("2227"), Some("anisearch"), false)));
    }

    #[test]
    fn titles_are_not_ids() {
        for title in ["Matrix", "Star Wars", "tt", "tt12", "tt0425622x", "300 Spartans", "", "unknown:5",
            "tmdb:abc", "imdb:603", "https://example.com/movie/5", "https://www.imdb.com/name/nm0000206/",
            "12345678901"]
        {
            assert_eq!(parse_id_query(title), None, "{title}");
        }
    }

    #[test]
    fn a_candidate_comes_from_the_lookup_fields() {
        let matched = ProviderMatch {
            fields: serde_json::json!({"title": " Lost ", "year": 2004, "overview": "Plane crash."}),
            poster_url: Some("https://img/p.jpg".into()),
            ..ProviderMatch::default()
        };
        let candidate = ProviderCandidate::from_match("4607", &matched).unwrap();
        assert_eq!(candidate.title, "Lost");
        assert_eq!(candidate.year, Some(2004));
        assert_eq!(candidate.overview, "Plane crash.");
        assert_eq!(candidate.provider_id, "4607");
        assert_eq!(candidate.poster_url.as_deref(), Some("https://img/p.jpg"));
        assert!(ProviderCandidate::from_match("1", &ProviderMatch::default()).is_none());
    }

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-plugin-api");
    }

    #[test]
    fn normalize_query_title_lowercases_and_turns_punctuation_into_boundaries() {
        assert_eq!(
            normalize_query_title("Movie: The Subtitle, Part One - Redux"),
            "movie the subtitle part one redux"
        );
    }

    #[test]
    fn normalize_query_title_drops_apostrophes_without_splitting_the_word() {
        assert_eq!(normalize_query_title("Ocean's Eleven"), "oceans eleven");
    }

    #[test]
    fn normalize_query_title_collapses_repeated_whitespace() {
        assert_eq!(normalize_query_title("Movie   Title"), "movie title");
    }

    #[test]
    fn strip_diacritics_folds_polish_and_czech_letters() {
        assert_eq!(
            strip_diacritics("asterix i obelix w służbie jej królewskiej mości"),
            "asterix i obelix w sluzbie jej krolewskiej mosci"
        );
        assert_eq!(
            strip_diacritics("příliš žluťoučký kůň"),
            "prilis zlutoucky kun"
        );
    }

    #[test]
    fn strip_diacritics_leaves_plain_ascii_untouched() {
        assert_eq!(strip_diacritics("plain title"), "plain title");
    }

    #[test]
    fn first_words_takes_only_the_requested_count() {
        assert_eq!(first_words("asterix i obelix w sluzbie", 2), "asterix i");
        assert_eq!(first_words("one", 2), "one");
        assert_eq!(first_words("", 2), "");
    }

    #[test]
    fn query_candidates_normalizes_only_once_for_a_plain_ascii_title() {
        assert_eq!(query_candidates("Inception"), vec!["inception"]);
    }

    #[test]
    fn query_candidates_adds_a_diacritics_stripped_variant_when_it_differs() {
        // Entry 0 is always the plain normalized (lowercased, punctuation-
        // cleaned) title — diacritics untouched; entry 1 is the
        // diacritics-stripped variant, only added because it differs.
        let candidates = query_candidates("Śluby Panieńskie");
        assert_eq!(candidates, vec!["śluby panieńskie", "sluby panienskie"]);

        let candidates = query_candidates("Żółć");
        assert_eq!(candidates, vec!["żółć", "zolc"]);
    }

    #[test]
    fn query_candidates_appends_a_two_word_fallback_for_a_longer_title() {
        // The exact real-world case this cascade exists for: TMDB rejected
        // the full, correctly-parsed title outright — this is the last
        // resort, tried only once both fuller attempts have failed.
        let candidates = query_candidates("Asterix i Obelix W Służbie Jej Królewskiej Mości");
        assert_eq!(
            candidates,
            vec![
                "asterix i obelix w służbie jej królewskiej mości",
                "asterix i obelix w sluzbie jej krolewskiej mosci",
                "asterix i",
            ]
        );
    }

    #[test]
    fn query_candidates_skips_the_fallback_when_it_duplicates_an_earlier_entry() {
        // A plain two-word ASCII title: the fallback (its own first two
        // words) is identical to the normalized entry already in the list.
        assert_eq!(query_candidates("Plain Title"), vec!["plain title"]);
    }
}
