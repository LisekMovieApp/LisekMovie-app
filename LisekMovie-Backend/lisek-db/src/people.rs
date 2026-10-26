//! Cast and crew (migration 0020): `people` and the `media_people` credits
//! linking them to items.
//!
//! Credits are written during metadata enrichment
//! (`lisek-server::metadata_enrichment`) through [`replace_credits`], which
//! only touches rows that actually changed — the six-hourly refresh rewrites
//! every item's credits, and an unchanged cast must not churn the table.
//! Every read applies the account's [`Restrictions`]: a person is visible
//! when at least one title crediting them is.

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::access::Restrictions;
use crate::catalog::CatalogRow;
use crate::{DbError, PgPool};

/// One credit as a provider reports it, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCredit {
    pub tmdb_id: i64,
    pub name: String,
    /// `"Actor"`, `"Director"` or `"Writer"`.
    pub role: String,
    pub character: Option<String>,
    pub order: i32,
}

/// A person a credit now points at, with the profile photo URL already on
/// disk (to compare against the provider's).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct LinkedPerson {
    pub id: Uuid,
    pub tmdb_id: i64,
    pub profile_url: Option<String>,
}

/// One entry per `(person, role)`: a provider listing the same actor twice
/// (two parts) becomes one credit with both characters, billed at the
/// earlier position. Order otherwise kept.
fn merge_credits(credits: &[NewCredit]) -> Vec<NewCredit> {
    let mut merged: Vec<NewCredit> = Vec::with_capacity(credits.len());
    let mut index: HashMap<(i64, &str), usize> = HashMap::new();
    for credit in credits {
        if credit.name.trim().is_empty() {
            continue;
        }
        match index.get(&(credit.tmdb_id, credit.role.as_str())) {
            Some(&at) => {
                let kept = &mut merged[at];
                kept.order = kept.order.min(credit.order);
                if let Some(extra) = credit.character.as_deref().filter(|c| !c.is_empty()) {
                    kept.character = Some(match kept.character.take() {
                        Some(known) if known.split(" / ").any(|c| c == extra) => known,
                        Some(known) => format!("{known} / {extra}"),
                        None => extra.to_string(),
                    });
                }
            }
            None => {
                index.insert((credit.tmdb_id, credit.role.as_str()), merged.len());
                merged.push(credit.clone());
            }
        }
    }
    merged
}

/// Makes `credits` the item's whole cast and crew: creates people seen for
/// the first time (or renames them), drops credits no longer listed and
/// writes only the ones that changed. Returns every person now credited.
pub async fn replace_credits(
    pool: &PgPool,
    item_id: Uuid,
    credits: &[NewCredit],
) -> Result<Vec<LinkedPerson>, DbError> {
    let credits = merge_credits(credits);

    // People, one row each, in id order so concurrent scans lock them in
    // the same order.
    let mut names: Vec<(i64, &str)> = credits.iter().map(|c| (c.tmdb_id, c.name.trim())).collect();
    names.sort_by_key(|(id, _)| *id);
    names.dedup_by_key(|(id, _)| *id);
    let tmdb_ids: Vec<i64> = names.iter().map(|(id, _)| *id).collect();
    let person_names: Vec<&str> = names.iter().map(|(_, name)| *name).collect();

    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO people (tmdb_id, name)
         SELECT * FROM unnest($1::bigint[], $2::text[])
         ON CONFLICT (tmdb_id) DO UPDATE SET name = EXCLUDED.name, updated_at = now()
         WHERE people.name IS DISTINCT FROM EXCLUDED.name",
    )
    .bind(&tmdb_ids)
    .bind(&person_names)
    .execute(&mut *tx)
    .await?;
    let people: Vec<LinkedPerson> = sqlx::query_as(
        "SELECT id, tmdb_id, profile_url FROM people WHERE tmdb_id = ANY($1) ORDER BY tmdb_id",
    )
    .bind(&tmdb_ids)
    .fetch_all(&mut *tx)
    .await?;
    let by_tmdb: HashMap<i64, Uuid> = people.iter().map(|p| (p.tmdb_id, p.id)).collect();

    let mut person_ids = Vec::with_capacity(credits.len());
    let mut roles = Vec::with_capacity(credits.len());
    let mut characters = Vec::with_capacity(credits.len());
    let mut orders = Vec::with_capacity(credits.len());
    for credit in &credits {
        let Some(&person_id) = by_tmdb.get(&credit.tmdb_id) else {
            continue;
        };
        person_ids.push(person_id);
        roles.push(credit.role.as_str());
        characters.push(credit.character.as_deref());
        orders.push(credit.order);
    }

    sqlx::query(
        "DELETE FROM media_people mp
         WHERE mp.media_item_id = $1
           AND NOT EXISTS (
               SELECT 1 FROM unnest($2::uuid[], $3::text[]) AS keep(person_id, role)
               WHERE keep.person_id = mp.person_id AND keep.role = mp.role)",
    )
    .bind(item_id)
    .bind(&person_ids)
    .bind(&roles)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO media_people (media_item_id, person_id, role, character_name, sort_order)
         SELECT $1, c.person_id, c.role, c.character_name, c.sort_order
         FROM unnest($2::uuid[], $3::text[], $4::text[], $5::int[])
              AS c(person_id, role, character_name, sort_order)
         ON CONFLICT (media_item_id, person_id, role) DO UPDATE SET
             character_name = EXCLUDED.character_name,
             sort_order = EXCLUDED.sort_order
         WHERE (media_people.character_name, media_people.sort_order)
               IS DISTINCT FROM (EXCLUDED.character_name, EXCLUDED.sort_order)",
    )
    .bind(item_id)
    .bind(&person_ids)
    .bind(&roles)
    .bind(&characters)
    .bind(&orders)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(people)
}

/// Records the profile photo now on disk (`None`: no photo).
pub async fn set_profile_url(
    pool: &PgPool,
    person_id: Uuid,
    url: Option<&str>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE people SET profile_url = $2, updated_at = now()
         WHERE id = $1 AND profile_url IS DISTINCT FROM $2",
    )
    .bind(person_id)
    .bind(url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes people no title credits any more. Returns their ids (their
/// photos go too).
pub async fn prune_orphans(pool: &PgPool) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "DELETE FROM people p
         WHERE NOT EXISTS (SELECT 1 FROM media_people mp WHERE mp.person_id = p.id)
         RETURNING p.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Every person id (the photo folder sweep).
pub async fn all_ids(pool: &PgPool) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM people")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// One credit on an item's page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ItemCredit {
    pub person_id: Uuid,
    pub name: String,
    pub role: String,
    pub character: Option<String>,
    /// A profile photo is on disk (`/api/people/{id}/image`).
    pub has_image: bool,
}

/// An item's credits: directors, writers, then the cast in billing order.
/// The caller checks the item is visible.
pub async fn item_credits(
    pool: &PgPool,
    item_id: Uuid,
    limit: i64,
) -> Result<Vec<ItemCredit>, DbError> {
    Ok(sqlx::query_as::<_, ItemCredit>(
        "SELECT p.id AS person_id, p.name, mp.role, mp.character_name AS character,
                p.profile_url IS NOT NULL AS has_image
         FROM media_people mp
         JOIN people p ON p.id = mp.person_id
         WHERE mp.media_item_id = $1
         ORDER BY CASE mp.role WHEN 'Director' THEN 0 WHEN 'Writer' THEN 1 ELSE 2 END,
                  mp.sort_order, p.name, p.id
         LIMIT $2",
    )
    .bind(item_id)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// A person as stored.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Person {
    pub id: Uuid,
    pub name: String,
    pub tmdb_id: i64,
    pub imdb_id: Option<String>,
    pub biography: Option<String>,
    pub birth_place: Option<String>,
    pub birth_date: Option<NaiveDate>,
    pub death_date: Option<NaiveDate>,
    pub profile_url: Option<String>,
    pub details_updated_at: Option<DateTime<Utc>>,
}

/// The person, when the account can see at least one title crediting
/// them. `None` otherwise (or no such person).
pub async fn get_visible(
    pool: &PgPool,
    person_id: Uuid,
    access: &Restrictions,
) -> Result<Option<Person>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    Ok(sqlx::query_as::<_, Person>(
        "SELECT p.id, p.name, p.tmdb_id, p.imdb_id, p.biography, p.birth_place,
                p.birth_date, p.death_date, p.profile_url, p.details_updated_at
         FROM people p
         WHERE p.id = $1
           AND EXISTS (
               SELECT 1 FROM media_people mp
               JOIN media_items m ON m.id = mp.media_item_id
               WHERE mp.person_id = p.id
                 AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4))",
    )
    .bind(person_id)
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .fetch_optional(pool)
    .await?)
}

/// Biography and dates from the provider's person page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PersonDetails {
    pub name: Option<String>,
    pub biography: Option<String>,
    pub birth_place: Option<String>,
    pub birth_date: Option<NaiveDate>,
    pub death_date: Option<NaiveDate>,
    pub imdb_id: Option<String>,
}

/// Saves the provider's details (an empty name keeps the stored one) and
/// marks them fresh. `details: None` only marks them checked (the provider
/// does not know the person), so the page does not ask again right away.
pub async fn save_details(
    pool: &PgPool,
    person_id: Uuid,
    details: Option<&PersonDetails>,
) -> Result<(), DbError> {
    match details {
        Some(d) => {
            sqlx::query(
                "UPDATE people SET
                     name = coalesce(nullif(btrim($2), ''), name),
                     biography = $3, birth_place = $4, birth_date = $5, death_date = $6,
                     imdb_id = $7, details_updated_at = now(), updated_at = now()
                 WHERE id = $1",
            )
            .bind(person_id)
            .bind(d.name.as_deref())
            .bind(d.biography.as_deref())
            .bind(d.birth_place.as_deref())
            .bind(d.birth_date)
            .bind(d.death_date)
            .bind(d.imdb_id.as_deref())
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query("UPDATE people SET details_updated_at = now() WHERE id = $1")
                .bind(person_id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// One title of a person's filmography: a card plus what they did on it.
#[derive(Debug, Clone)]
pub struct FilmographyEntry {
    pub item: lisek_core::MediaItem,
    /// Distinct roles (`"Actor"`, `"Director"`, `"Writer"`).
    pub roles: Vec<String>,
    pub character: Option<String>,
}

#[derive(sqlx::FromRow)]
struct FilmographyRow {
    #[sqlx(flatten)]
    card: CatalogRow,
    roles: Vec<String>,
    character: Option<String>,
}

/// The visible titles crediting a person — films one by one, a series once
/// (its card, however many episodes credit them) — newest first, undated
/// last.
pub async fn filmography(
    pool: &PgPool,
    person_id: Uuid,
    access: &Restrictions,
    limit: i64,
) -> Result<Vec<FilmographyEntry>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    let rows = sqlx::query_as::<_, FilmographyRow>(
        "WITH credits AS (
             SELECT mp.media_item_id, mp.role, mp.character_name
             FROM media_people mp WHERE mp.person_id = $1
         )
         SELECT * FROM (
             (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                     NULL::int AS season, NULL::int AS episode, m.created_at,
                     array_agg(DISTINCT c.role) AS roles,
                     min(c.character_name) AS character
              FROM credits c
              JOIN media_items m ON m.id = c.media_item_id
              WHERE m.item_type = 'movie'
                AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
              GROUP BY m.id)
             UNION ALL
             (SELECT r.id, s.library_id, 'series', s.title, NULL::int, r.overview,
                     NULL::int, NULL::int, s.last_episode_added_at,
                     array_agg(DISTINCT c.role),
                     min(c.character_name)
              FROM credits c
              JOIN episodes e ON e.media_item_id = c.media_item_id
              JOIN series s ON s.id = e.series_id
              JOIN media_items r ON r.id = s.representative_item_id
              WHERE lisek_visible(s.library_id, r.age_rating, $2, $3, $4)
              GROUP BY s.id, r.id)
         ) f
         ORDER BY f.year DESC NULLS LAST, f.title, f.id
         LIMIT $5",
    )
    .bind(person_id)
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| FilmographyEntry {
            item: row.card.into(),
            roles: row.roles,
            character: row.character,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn credit(tmdb_id: i64, role: &str, character: Option<&str>, order: i32) -> NewCredit {
        NewCredit {
            tmdb_id,
            name: format!("Person {tmdb_id}"),
            role: role.to_string(),
            character: character.map(str::to_string),
            order,
        }
    }

    #[test]
    fn a_double_role_becomes_one_credit_with_both_characters() {
        let merged = merge_credits(&[
            credit(1, "Actor", Some("Jekyll"), 3),
            credit(2, "Actor", None, 1),
            credit(1, "Actor", Some("Hyde"), 0),
            credit(1, "Director", None, 0),
            credit(1, "Actor", Some("Hyde"), 5),
        ]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].character.as_deref(), Some("Jekyll / Hyde"));
        assert_eq!(merged[0].order, 0);
        assert_eq!(merged[2].role, "Director");
    }

    #[test]
    fn nameless_credits_are_dropped() {
        let mut nameless = credit(3, "Actor", None, 0);
        nameless.name = "  ".into();
        assert!(merge_credits(&[nameless]).is_empty());
    }
}
