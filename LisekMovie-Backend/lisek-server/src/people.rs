//! Cast and crew (Faza 8): an item's credits, a person's page with their
//! filmography from this server's libraries, and profile photos.
//!
//! Credits and photos are stored during enrichment (`metadata_enrichment`).
//! A person's biography and dates are read from the provider here, the
//! first time the page is opened and again once they are a month old —
//! scans stay as fast as before, and people nobody looks at cost nothing.
//! Every endpoint applies the account's restrictions: a person is visible
//! when at least one title crediting them is.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::Response;
use chrono::{Datelike, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

use lisek_db::people::{self as db, Person, PersonDetails};

use crate::auth::AuthUser;
use crate::media::{MediaError, MediaItemResponse, ensure_visible, image_response, to_responses};
use crate::metadata_enrichment::{person_image_path, sync_artwork};
use crate::provider_registry;
use crate::state::{SharedState, config_dir};

/// Credits shown on an item's page.
const ITEM_CREDIT_LIMIT: i64 = 60;
/// Titles listed on a person's page.
const FILMOGRAPHY_LIMIT: i64 = 200;
/// Provider details older than this are read again.
const DETAILS_MAX_AGE: chrono::TimeDelta = chrono::TimeDelta::days(30);
/// The page never waits longer than this for the provider.
const DETAILS_TIMEOUT: Duration = Duration::from_secs(8);

/// One person credited on an item.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CreditResponse {
    pub person_id: Uuid,
    pub name: String,
    /// `"Actor"`, `"Director"` or `"Writer"`.
    pub role: String,
    /// The part an actor plays.
    pub character: Option<String>,
    /// `/api/people/{id}/image`, when there is a photo.
    pub image_url: Option<String>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ItemPeopleResponse {
    /// Directors, writers, then the cast in billing order.
    pub items: Vec<CreditResponse>,
}

fn image_url(person_id: Uuid, has_image: bool) -> Option<String> {
    has_image.then(|| format!("/api/people/{person_id}/image"))
}

/// The cast and crew of a film or series (a series card's id is one of its
/// episodes; every episode carries the series' credits).
#[utoipa::path(get,path="/api/media/{item_id}/people",tag="media",params(("item_id"=Uuid,Path)),responses((status=200,body=ItemPeopleResponse),(status=401),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn item_people(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(item_id): Path<Uuid>,
) -> Result<Json<ItemPeopleResponse>, MediaError> {
    let pool = state.pool().ok_or(MediaError::ServerNotReady)?;
    ensure_visible(&pool, &auth, item_id).await?;
    let credits = db::item_credits(&pool, item_id, ITEM_CREDIT_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    Ok(Json(ItemPeopleResponse {
        items: credits
            .into_iter()
            .map(|c| CreditResponse {
                image_url: image_url(c.person_id, c.has_image),
                person_id: c.person_id,
                name: c.name,
                role: c.role,
                character: c.character,
            })
            .collect(),
    }))
}

/// One title of a person's filmography.
#[derive(Serialize, utoipa::ToSchema)]
pub struct FilmographyItemResponse {
    pub item: MediaItemResponse,
    /// What they did on it: `"Actor"`, `"Director"`, `"Writer"`.
    pub roles: Vec<String>,
    pub character: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PersonResponse {
    pub id: Uuid,
    pub name: String,
    pub biography: Option<String>,
    pub birth_date: Option<NaiveDate>,
    pub death_date: Option<NaiveDate>,
    pub birth_place: Option<String>,
    /// Age today, or at death.
    pub age: Option<i32>,
    pub image_url: Option<String>,
    pub tmdb_url: String,
    pub imdb_url: Option<String>,
    /// Titles in this server's libraries the account can see, newest first.
    pub filmography: Vec<FilmographyItemResponse>,
}

/// Full years between two dates.
fn years_between(from: NaiveDate, to: NaiveDate) -> Option<i32> {
    let mut years = to.year() - from.year();
    if (to.month(), to.day()) < (from.month(), from.day()) {
        years -= 1;
    }
    (years >= 0).then_some(years)
}

fn age(person: &Person, today: NaiveDate) -> Option<i32> {
    years_between(person.birth_date?, person.death_date.unwrap_or(today))
}

fn parse_date(value: Option<&str>) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value?, "%Y-%m-%d").ok()
}

fn details_are_stale(person: &Person) -> bool {
    person
        .details_updated_at
        .is_none_or(|at| Utc::now() - at > DETAILS_MAX_AGE)
}

/// Reads the person's page from TMDB and stores it, with the larger
/// profile photo. Any failure leaves the stored data as it is.
async fn refresh_details(
    state: &SharedState,
    pool: &lisek_db::PgPool,
    person: &Person,
    language: &str,
) {
    let api_key = match provider_registry::enabled_provider_keys(pool, &state.secrets).await {
        Ok(keys) => keys
            .into_iter()
            .find(|(key, _)| key == "tmdb")
            .map(|(_, key)| key),
        Err(err) => {
            tracing::warn!(%err, "could not read the provider keys");
            return;
        }
    };
    let Some(provider) = api_key.and_then(|key| provider_registry::build_provider("tmdb", &key))
    else {
        tracing::debug!(person_id = %person.id, "TMDB not configured: person details stay as stored");
        return;
    };
    let tmdb_id = person.tmdb_id.to_string();
    let found =
        match tokio::time::timeout(DETAILS_TIMEOUT, provider.person(&tmdb_id, language)).await {
            Ok(Ok(found)) => found,
            Ok(Err(err)) => {
                tracing::warn!(person_id = %person.id, %err, "person details lookup failed");
                return;
            }
            Err(_) => {
                tracing::warn!(person_id = %person.id, "person details lookup timed out");
                return;
            }
        };
    let details = found.as_ref().map(|d| PersonDetails {
        name: Some(d.name.clone()),
        biography: Some(d.biography.clone()).filter(|b| !b.is_empty()),
        birth_place: d.birth_place.clone(),
        birth_date: parse_date(d.birth_date.as_deref()),
        death_date: parse_date(d.death_date.as_deref()),
        imdb_id: d.imdb_id.clone(),
    });
    if let Err(err) = db::save_details(pool, person.id, details.as_ref()).await {
        tracing::warn!(person_id = %person.id, %err, "failed to save person details");
        return;
    }
    // The page shows the photo large: fetch the provider's bigger size.
    if let Some(wanted) = found.as_ref().and_then(|d| d.profile_url.as_deref())
        && person.profile_url.as_deref() != Some(wanted)
    {
        let path = person_image_path(&config_dir(), person.id);
        let now = sync_artwork(
            &reqwest::Client::new(),
            &path,
            &tmdb_id,
            person.profile_url.clone(),
            Some(wanted),
        )
        .await;
        if now != person.profile_url
            && let Err(err) = db::set_profile_url(pool, person.id, now.as_deref()).await
        {
            tracing::warn!(person_id = %person.id, %err, "failed to record a profile photo");
        }
    }
}

/// A person's profile: biography, dates, links and the titles of this
/// server crediting them.
#[utoipa::path(get,path="/api/people/{person_id}",tag="media",params(("person_id"=Uuid,Path)),responses((status=200,body=PersonResponse),(status=401),(status=404),(status=500),(status=503)),security(("lisek_auth"=[])))]
pub async fn get_person(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(person_id): Path<Uuid>,
) -> Result<Json<PersonResponse>, MediaError> {
    let pool = state.pool().ok_or(MediaError::ServerNotReady)?;
    let visible = || db::get_visible(&pool, person_id, &auth.access);
    let mut person = visible()
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::NotFound)?;
    if details_are_stale(&person) {
        refresh_details(&state, &pool, &person, &auth.user.ui_language).await;
        person = visible()
            .await
            .map_err(|_| MediaError::Internal)?
            .ok_or(MediaError::NotFound)?;
    }
    let entries = db::filmography(&pool, person_id, &auth.access, FILMOGRAPHY_LIMIT)
        .await
        .map_err(|_| MediaError::Internal)?;
    let (cards, credits): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .map(|e| (e.item, (e.roles, e.character)))
        .unzip();
    let filmography = to_responses(&pool, cards)
        .await
        .into_iter()
        .zip(credits)
        .map(|(item, (roles, character))| FilmographyItemResponse {
            item,
            roles,
            character,
        })
        .collect();
    Ok(Json(PersonResponse {
        age: age(&person, Utc::now().date_naive()),
        image_url: image_url(person.id, person.profile_url.is_some()),
        tmdb_url: format!("https://www.themoviedb.org/person/{}", person.tmdb_id),
        imdb_url: person
            .imdb_id
            .as_deref()
            .filter(|id| id.starts_with("nm") && id[2..].chars().all(|c| c.is_ascii_digit()))
            .map(|id| format!("https://www.imdb.com/name/{id}/")),
        id: person.id,
        name: person.name,
        biography: person.biography,
        birth_date: person.birth_date,
        death_date: person.death_date,
        birth_place: person.birth_place,
        filmography,
    }))
}

/// A person's profile photo (`?token=` works, as for posters).
#[utoipa::path(get,path="/api/people/{person_id}/image",tag="media",params(("person_id"=Uuid,Path)),responses((status=200,description="Image bytes"),(status=401),(status=404),(status=503)),security(("lisek_auth"=[])))]
pub async fn person_image(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(person_id): Path<Uuid>,
) -> Result<Response, MediaError> {
    let pool = state.pool().ok_or(MediaError::ServerNotReady)?;
    db::get_visible(&pool, person_id, &auth.access)
        .await
        .map_err(|_| MediaError::Internal)?
        .ok_or(MediaError::NotFound)?;
    let bytes = tokio::fs::read(person_image_path(&config_dir(), person_id))
        .await
        .map_err(|_| MediaError::NotFound)?;
    Ok(image_response(bytes, "image/jpeg"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn age_counts_full_years_up_to_today_or_death() {
        assert_eq!(
            years_between(date(1942, 7, 13), date(2026, 7, 12)),
            Some(83)
        );
        assert_eq!(
            years_between(date(1942, 7, 13), date(2026, 7, 13)),
            Some(84)
        );
        assert_eq!(years_between(date(2000, 2, 29), date(2001, 2, 28)), Some(0));
        assert_eq!(years_between(date(2030, 1, 1), date(2026, 1, 1)), None);
        let mut person = Person {
            id: Uuid::nil(),
            name: "Heath Ledger".into(),
            tmdb_id: 1810,
            imdb_id: None,
            biography: None,
            birth_place: None,
            birth_date: Some(date(1979, 4, 4)),
            death_date: Some(date(2008, 1, 22)),
            profile_url: None,
            details_updated_at: None,
        };
        assert_eq!(age(&person, date(2026, 10, 26)), Some(28));
        assert!(details_are_stale(&person));
        person.details_updated_at = Some(Utc::now());
        assert!(!details_are_stale(&person));
        person.birth_date = None;
        assert_eq!(age(&person, date(2026, 10, 26)), None);
    }

    #[test]
    fn dates_are_iso() {
        assert_eq!(parse_date(Some("1956-03-07")), Some(date(1956, 3, 7)));
        assert_eq!(parse_date(Some("07.03.1956")), None);
        assert_eq!(parse_date(None), None);
    }
}
