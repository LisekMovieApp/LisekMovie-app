//! Faza 21: the metadata "armour". This module holds the pure rules, and
//! [`crate::metadata_enrichment`] runs them against the plugins.
//!
//! * **Language chain.** The administrator orders languages (`pl`, `en`,
//!   `original`, `any` ...). A match is fetched in the first language. If
//!   its title, synopsis or tagline (or a season's name or synopsis) is
//!   empty, the same title is fetched again from the same plugin by its
//!   own id in the next language. Each text field then comes from the
//!   first language in the order that has it.
//! * **Provider cascade.** If the best plugin's match still has gaps in
//!   the key fields (synopsis, poster, genres and the others in
//!   [`CASCADE_FIELDS`]), the next plugins in the administrator's priority
//!   order fill them. A match from another plugin is used only when it is
//!   the same title: no shared id may differ, and the years may differ by
//!   one at most. `fields.filled_from` records which plugin gave which
//!   field, and `fields.language_from` which language gave which text.

use std::collections::HashSet;

use lisek_plugin_api::ProviderMatch;
use serde_json::{Map, Value};

/// Chain entry: the production's own language (`fields.original_language`).
pub const ORIGINAL: &str = "original";
/// Chain entry: whatever is left (English when not listed, then the
/// original title as a last resort for the title).
pub const ANY: &str = "any";

/// Text that depends on the language asked for.
pub const TEXT_FIELDS: &[&str] = &["title", "overview", "tagline"];

/// Fields a later plugin may fill when the best one lacks them. `poster`,
/// `backdrop`, `logo`, `people` and `seasons` are the match's own parts,
/// the rest are keys of `fields`.
pub const CASCADE_FIELDS: &[&str] = &[
    "overview",
    "poster",
    "genres",
    "backdrop",
    "logo",
    "tagline",
    "rating",
    "runtime_minutes",
    "original_title",
    "original_language",
    "studios",
    "tags",
    "certification",
    "age_rating",
    "year",
    "people",
    "seasons",
];

/// The fields that make a match worth asking the next plugin about.
pub const KEY_FIELDS: &[&str] = &["overview", "poster", "genres"];

/// Ids that name the same title across plugins.
const SHARED_IDS: &[&str] = &[
    "tmdb_id",
    "tvdb_id",
    "imdb_id",
    "anidb_id",
    "anilist_id",
    "mal_id",
];

/// Most entries the administrator may list.
pub const MAX_LANGUAGES: usize = 12;

fn blank(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.is_empty(),
        Some(_) => false,
    }
}

fn blank_str(value: Option<&str>) -> bool {
    value.is_none_or(|s| s.trim().is_empty())
}

/// Whether `field` (one of [`CASCADE_FIELDS`]) is missing in `matched`.
pub fn is_missing(matched: &ProviderMatch, field: &str) -> bool {
    match field {
        "poster" => blank_str(matched.poster_url.as_deref()),
        "backdrop" => blank_str(matched.backdrop_url.as_deref()),
        "logo" => blank_str(matched.logo_url.as_deref()),
        "people" => matched.people.is_none(),
        "seasons" => matched.seasons.is_empty(),
        key => blank(matched.fields.get(key)),
    }
}

/// The key fields `matched` lacks (`overview`, `poster`, `genres`).
pub fn missing_key_fields(matched: &ProviderMatch) -> Vec<&'static str> {
    KEY_FIELDS
        .iter()
        .copied()
        .filter(|field| is_missing(matched, field))
        .collect()
}

/// Whether the match needs the next language: its synopsis is empty, its
/// title is sent but empty (`find_*` matches send none, the scanner's
/// title stays), or a season has no name. Taglines and season synopses
/// are filled on the way when another language is fetched anyway, but
/// often exist in no language, so they never cost a request alone.
pub fn needs_language(matched: &ProviderMatch) -> bool {
    blank(matched.fields.get("overview"))
        || matched.fields.get("title").is_some_and(|t| blank(Some(t)))
        || matched.seasons.iter().any(|s| blank_str(s.name.as_deref()))
}

fn year_of(fields: &Value) -> Option<i64> {
    match fields.get("year")? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.get(..4)?.parse().ok(),
        _ => None,
    }
}

fn id_of(fields: &Value, key: &str) -> Option<String> {
    match fields.get(key)? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_ascii_lowercase()),
        _ => None,
    }
}

/// Whether `other` (another plugin's match for the same query) names the
/// same title as `primary`: no id both know may differ, and the release
/// years may be one apart at most (a film's festival year, a series
/// premiering on New Year's Eve).
pub fn consistent(primary: &Value, other: &Value) -> bool {
    for key in SHARED_IDS {
        if let (Some(a), Some(b)) = (id_of(primary, key), id_of(other, key))
            && a != b
        {
            return false;
        }
    }
    match (year_of(primary), year_of(other)) {
        (Some(a), Some(b)) => (a - b).abs() <= 1,
        _ => true,
    }
}

fn note(fields: &mut Value, record: &str, field: &str, from: &str) {
    if !fields.is_object() {
        *fields = Value::Object(Map::new());
    }
    let object = fields.as_object_mut().expect("object");
    let entry = object
        .entry(record.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    entry
        .as_object_mut()
        .expect("object")
        .insert(field.to_owned(), Value::String(from.to_owned()));
}

fn set_field(fields: &mut Value, key: &str, value: Value) {
    if !fields.is_object() {
        *fields = Value::Object(Map::new());
    }
    fields
        .as_object_mut()
        .expect("object")
        .insert(key.to_owned(), value);
}

/// Fills `primary`'s gaps in [`CASCADE_FIELDS`] from `other` (a match
/// from the plugin `source`), plus ids `primary` does not know. Seasons
/// `primary` has get only their missing names, synopses and posters.
/// Returns the fields that were filled. The caller checks
/// [`consistent`] first.
pub fn fill_from(primary: &mut ProviderMatch, other: &ProviderMatch, source: &str) -> Vec<&'static str> {
    let mut filled = Vec::new();
    for field in CASCADE_FIELDS {
        if !is_missing(primary, field) || is_missing(other, field) {
            continue;
        }
        match *field {
            "poster" => primary.poster_url = other.poster_url.clone(),
            "backdrop" => primary.backdrop_url = other.backdrop_url.clone(),
            "logo" => primary.logo_url = other.logo_url.clone(),
            "people" => primary.people = other.people.clone(),
            "seasons" => primary.seasons = other.seasons.clone(),
            key => {
                let value = other.fields.get(key).cloned().unwrap_or(Value::Null);
                set_field(&mut primary.fields, key, value);
            }
        }
        note(&mut primary.fields, "filled_from", field, source);
        filled.push(*field);
    }
    for season in &mut primary.seasons {
        let Some(theirs) = other.seasons.iter().find(|s| s.number == season.number) else {
            continue;
        };
        let mut any = false;
        if blank_str(season.name.as_deref()) && !blank_str(theirs.name.as_deref()) {
            season.name = theirs.name.clone();
            any = true;
        }
        if blank_str(season.overview.as_deref()) && !blank_str(theirs.overview.as_deref()) {
            season.overview = theirs.overview.clone();
            any = true;
        }
        if blank_str(season.poster_url.as_deref()) && !blank_str(theirs.poster_url.as_deref()) {
            season.poster_url = theirs.poster_url.clone();
            any = true;
        }
        if season.air_year.is_none() && theirs.air_year.is_some() {
            season.air_year = theirs.air_year;
        }
        if any && !filled.contains(&"seasons") {
            note(&mut primary.fields, "filled_from", "seasons", source);
            filled.push("seasons");
        }
    }
    for key in SHARED_IDS {
        if blank(primary.fields.get(*key)) && !blank(other.fields.get(*key)) {
            let value = other.fields.get(*key).cloned().unwrap_or(Value::Null);
            set_field(&mut primary.fields, key, value);
        }
    }
    filled
}

/// A match found by title carries no `title` (the scanner's stays); a
/// later language must not add one.
fn skip_title(base: &ProviderMatch, field: &str) -> bool {
    field == "title" && base.fields.get("title").is_none()
}

/// Fills `base`'s empty language-dependent text from `other` (the same
/// title from the same plugin, fetched in `language`). Returns whether
/// anything was filled.
pub fn fill_text(base: &mut ProviderMatch, other: &ProviderMatch, language: &str) -> bool {
    let mut any = false;
    for field in TEXT_FIELDS {
        if skip_title(base, field) {
            continue;
        }
        if blank(base.fields.get(*field)) && !blank(other.fields.get(*field)) {
            let value = other.fields.get(*field).cloned().unwrap_or(Value::Null);
            set_field(&mut base.fields, field, value);
            note(&mut base.fields, "language_from", field, language);
            any = true;
        }
    }
    for season in &mut base.seasons {
        let Some(theirs) = other.seasons.iter().find(|s| s.number == season.number) else {
            continue;
        };
        if blank_str(season.name.as_deref()) && !blank_str(theirs.name.as_deref()) {
            season.name = theirs.name.clone();
            any = true;
        }
        if blank_str(season.overview.as_deref()) && !blank_str(theirs.overview.as_deref()) {
            season.overview = theirs.overview.clone();
            any = true;
        }
    }
    any
}

/// The last resort of `any`: an empty title becomes the original title.
pub fn title_from_original(matched: &mut ProviderMatch) {
    if blank(matched.fields.get("title")) && !blank(matched.fields.get("original_title")) {
        let value = matched.fields.get("original_title").cloned().unwrap_or(Value::Null);
        set_field(&mut matched.fields, "title", value);
        note(&mut matched.fields, "language_from", "title", ORIGINAL);
    }
}

/// Puts the text fields `wanted` (the same title in a language the
/// administrator ranks higher than the one `base` was fetched in) over
/// `base`'s. Used when the chain starts with `original`.
pub fn prefer_text(base: &mut ProviderMatch, wanted: &ProviderMatch, language: &str) {
    for field in TEXT_FIELDS {
        if skip_title(base, field) {
            continue;
        }
        if !blank(wanted.fields.get(*field)) {
            let value = wanted.fields.get(*field).cloned().unwrap_or(Value::Null);
            set_field(&mut base.fields, field, value);
            note(&mut base.fields, "language_from", field, language);
        }
    }
    for season in &mut base.seasons {
        let Some(theirs) = wanted.seasons.iter().find(|s| s.number == season.number) else {
            continue;
        };
        if !blank_str(theirs.name.as_deref()) {
            season.name = theirs.name.clone();
        }
        if !blank_str(theirs.overview.as_deref()) {
            season.overview = theirs.overview.clone();
        }
    }
}

/// A normalized language code (`pl`, `pt-br`), or `None`.
pub fn normalize_code(code: &str) -> Option<String> {
    let code = code.trim().to_ascii_lowercase();
    if code == ORIGINAL || code == ANY {
        return Some(code);
    }
    let mut parts = code.split('-');
    let main = parts.next()?;
    let region = parts.next();
    if parts.next().is_some() || !(2..=3).contains(&main.len()) || !main.bytes().all(|b| b.is_ascii_lowercase()) {
        return None;
    }
    match region {
        None => Some(main.to_owned()),
        Some(r) if (2..=4).contains(&r.len()) && r.bytes().all(|b| b.is_ascii_alphanumeric()) => {
            Some(format!("{main}-{r}"))
        }
        Some(_) => None,
    }
}

/// Validates an order the administrator sent: known shape, no
/// duplicates, at most [`MAX_LANGUAGES`]. `Err` names the bad entry.
pub fn validate(entries: &[String]) -> Result<Vec<String>, String> {
    if entries.len() > MAX_LANGUAGES {
        return Err(format!("at most {MAX_LANGUAGES} languages"));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        let Some(code) = normalize_code(entry) else {
            return Err(entry.clone());
        };
        if seen.insert(code.clone()) {
            out.push(code);
        }
    }
    Ok(out)
}

/// The order in effect: the stored one, or the server's default language,
/// then English, then the original language.
pub fn effective_chain(stored: &[String], default_language: &str) -> Vec<String> {
    let cleaned: Vec<String> = stored.iter().filter_map(|e| normalize_code(e)).collect();
    if !cleaned.is_empty() {
        let mut seen = HashSet::new();
        return cleaned.into_iter().filter(|c| seen.insert(c.clone())).collect();
    }
    let mut chain = vec![default_language.to_owned()];
    if default_language != "en" {
        chain.push("en".to_owned());
    }
    chain.push(ORIGINAL.to_owned());
    chain
}

/// The language a match is first fetched in: the first real code in the
/// chain (before the original language is known), else `fallback`.
pub fn first_language(chain: &[String], fallback: &str) -> String {
    chain
        .iter()
        .find(|e| *e != ORIGINAL && *e != ANY)
        .cloned()
        .unwrap_or_else(|| fallback.to_owned())
}

/// The chain as real codes once the production's language is known:
/// `original` becomes it (or is dropped), `any` becomes English when
/// English is not listed (or is dropped). Duplicates go.
pub fn resolve_chain(chain: &[String], original_language: Option<&str>) -> Vec<String> {
    let listed: HashSet<&str> = chain.iter().map(String::as_str).collect();
    let mut out: Vec<String> = Vec::new();
    for entry in chain {
        let code = match entry.as_str() {
            ORIGINAL => original_language.and_then(normalize_code),
            ANY => (!listed.contains("en")).then(|| "en".to_owned()),
            code => Some(code.to_owned()),
        };
        if let Some(code) = code
            && code != ORIGINAL
            && code != ANY
            && !out.contains(&code)
        {
            out.push(code);
        }
    }
    out
}

/// The production's own language as a plugin reported it.
pub fn original_language(matched: &ProviderMatch) -> Option<String> {
    matched
        .fields
        .get("original_language")
        .and_then(Value::as_str)
        .and_then(normalize_code)
}

/// The plugin's own id for the title (`tmdb_id` for `tmdb` ...).
pub fn own_id(matched: &ProviderMatch, provider_key: &str) -> Option<String> {
    match matched.fields.get(format!("{provider_key}_id"))? {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lisek_plugin_api::ProviderSeason;
    use serde_json::json;

    fn matched(fields: Value) -> ProviderMatch {
        ProviderMatch {
            fields,
            ..ProviderMatch::default()
        }
    }

    #[test]
    fn the_default_chain_is_server_language_english_original() {
        assert_eq!(effective_chain(&[], "pl"), vec!["pl", "en", "original"]);
        assert_eq!(effective_chain(&[], "en"), vec!["en", "original"]);
        let stored = vec!["PL".to_owned(), "original".to_owned(), "pl".to_owned(), "bad code".to_owned()];
        assert_eq!(effective_chain(&stored, "en"), vec!["pl", "original"]);
    }

    #[test]
    fn original_and_any_resolve_once_the_language_is_known() {
        let chain: Vec<String> = ["pl", "en", "original", "any"].map(String::from).to_vec();
        assert_eq!(resolve_chain(&chain, Some("ja")), vec!["pl", "en", "ja"]);
        assert_eq!(resolve_chain(&chain, Some("pl")), vec!["pl", "en"]);
        assert_eq!(resolve_chain(&chain, None), vec!["pl", "en"]);
        let chain: Vec<String> = ["original", "pl", "any"].map(String::from).to_vec();
        assert_eq!(first_language(&chain, "en"), "pl");
        assert_eq!(resolve_chain(&chain, Some("ko")), vec!["ko", "pl", "en"]);
    }

    #[test]
    fn validation_rejects_junk_and_drops_duplicates() {
        let ok = validate(&["pl".into(), "pt-BR".into(), "original".into(), "pl".into(), "any".into()]).unwrap();
        assert_eq!(ok, vec!["pl", "pt-br", "original", "any"]);
        assert_eq!(validate(&["polski".into()]), Err("polski".to_owned()));
        assert!(validate(&vec!["pl".to_owned(); MAX_LANGUAGES + 1]).is_err());
    }

    #[test]
    fn empty_text_is_filled_from_the_next_language_only() {
        assert!(!needs_language(&matched(json!({"overview": "Ok", "tagline": ""}))));
        assert!(needs_language(&matched(json!({"overview": "Ok", "title": ""}))));
        let mut base = matched(json!({"title": "Diuna", "overview": "", "tagline": null}));
        base.seasons = vec![ProviderSeason { number: 1, name: Some("Sezon 1".into()), overview: None, ..Default::default() }];
        assert!(needs_language(&base));
        let mut english = matched(json!({"title": "Dune", "overview": "Paul Atreides...", "tagline": "Fear is the mind-killer."}));
        english.seasons = vec![ProviderSeason { number: 1, name: Some("Season 1".into()), overview: Some("First".into()), ..Default::default() }];
        assert!(fill_text(&mut base, &english, "en"));
        assert_eq!(base.fields["title"], "Diuna");
        assert_eq!(base.fields["overview"], "Paul Atreides...");
        assert_eq!(base.fields["language_from"]["overview"], "en");
        assert_eq!(base.fields["language_from"]["tagline"], "en");
        assert!(base.fields["language_from"].get("title").is_none());
        assert_eq!(base.seasons[0].name.as_deref(), Some("Sezon 1"));
        assert_eq!(base.seasons[0].overview.as_deref(), Some("First"));
        assert!(!needs_language(&base));
        assert!(!fill_text(&mut base, &english, "en"));
    }

    #[test]
    fn a_higher_ranked_language_wins_over_the_fetched_one() {
        let mut base = matched(json!({"title": "Spirited Away", "overview": "A girl..."}));
        let japanese = matched(json!({"title": "千と千尋の神隠し", "overview": ""}));
        prefer_text(&mut base, &japanese, "ja");
        assert_eq!(base.fields["title"], "千と千尋の神隠し");
        assert_eq!(base.fields["overview"], "A girl...");
    }

    #[test]
    fn the_original_title_is_the_last_resort() {
        let mut base = matched(json!({"title": "", "original_title": "Amélie"}));
        title_from_original(&mut base);
        assert_eq!(base.fields["title"], "Amélie");
    }

    #[test]
    fn only_the_same_title_from_another_plugin_is_used() {
        let primary = json!({"tmdb_id": 438631, "imdb_id": "tt1160419", "year": 2021});
        assert!(consistent(&primary, &json!({"imdb_id": "TT1160419", "year": 2021, "tvdb_id": 1})));
        assert!(consistent(&primary, &json!({"year": 2020})));
        assert!(consistent(&primary, &json!({})));
        assert!(!consistent(&primary, &json!({"imdb_id": "tt0087182", "year": 1984})));
        assert!(!consistent(&primary, &json!({"tmdb_id": 841})));
        assert!(!consistent(&primary, &json!({"year": "1984-12-14"})));
    }

    #[test]
    fn gaps_are_filled_from_the_next_plugin_and_recorded() {
        let mut primary = matched(json!({"tvdb_id": 81189, "title": "Breaking Bad", "overview": "", "genres": []}));
        primary.backdrop_url = Some("https://tvdb/backdrop.jpg".into());
        primary.seasons = vec![ProviderSeason { number: 1, name: Some("Season 1".into()), ..Default::default() }];
        assert_eq!(missing_key_fields(&primary), vec!["overview", "poster", "genres"]);
        let mut other = matched(json!({
            "tmdb_id": 1396, "tvdb_id": 81189, "title": "Breaking Bad: other",
            "overview": "A chemistry teacher...", "genres": ["Drama"], "rating": 8.9
        }));
        other.poster_url = Some("https://tmdb/poster.jpg".into());
        other.backdrop_url = Some("https://tmdb/backdrop.jpg".into());
        other.people = Some(Vec::new());
        other.seasons = vec![ProviderSeason { number: 1, overview: Some("Walter...".into()), poster_url: Some("https://tmdb/s1.jpg".into()), ..Default::default() }];
        let filled = fill_from(&mut primary, &other, "tmdb");
        assert_eq!(filled, vec!["overview", "poster", "genres", "rating", "people", "seasons"]);
        assert_eq!(primary.fields["title"], "Breaking Bad");
        assert_eq!(primary.backdrop_url.as_deref(), Some("https://tvdb/backdrop.jpg"));
        assert_eq!(primary.poster_url.as_deref(), Some("https://tmdb/poster.jpg"));
        assert_eq!(primary.fields["genres"], json!(["Drama"]));
        assert_eq!(primary.fields["tmdb_id"], 1396);
        assert_eq!(primary.fields["filled_from"]["overview"], "tmdb");
        assert_eq!(primary.seasons[0].name.as_deref(), Some("Season 1"));
        assert_eq!(primary.seasons[0].overview.as_deref(), Some("Walter..."));
        assert!(missing_key_fields(&primary).is_empty());
    }

    #[test]
    fn the_own_id_is_read_by_plugin_key() {
        let m = matched(json!({"tmdb_id": 603, "tvdb_id": "", "imdb_id": "tt0133093"}));
        assert_eq!(own_id(&m, "tmdb").as_deref(), Some("603"));
        assert_eq!(own_id(&m, "tvdb"), None);
        assert_eq!(own_id(&m, "imdb").as_deref(), Some("tt0133093"));
        assert_eq!(original_language(&matched(json!({"original_language": "JA"}))).as_deref(), Some("ja"));
    }
}
