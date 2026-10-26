//! Global search (Faza 11): one query string against film and series
//! titles (and original titles, genres, tags), episode titles, people and
//! collections, each group from its own trigram index (migration 0023) and
//! under the account's [`Restrictions`].
//!
//! Matching is `ILIKE '%query%'` — answered by the GIN trigram indexes for
//! queries of three characters or more — ranked by: the title starts with
//! the query, then trigram `similarity`, then the title itself. A match
//! on genres/tags only ranks after title matches.

use uuid::Uuid;

use lisek_core::MediaItem;

use crate::access::Restrictions;
use crate::catalog::{CatalogRow, escape_like, into_items};
use crate::{DbError, PgPool};

/// The `ILIKE` patterns for a query: `(%query%, query%)`, with `%`, `_`
/// and `\` in the query matched literally.
pub fn patterns(query: &str) -> (String, String) {
    let escaped = escape_like(query.trim());
    (format!("%{escaped}%"), format!("{escaped}%"))
}

/// Films matching the query, best first.
pub async fn movies(
    pool: &PgPool,
    query: &str,
    access: &Restrictions,
    limit: i64,
) -> Result<Vec<MediaItem>, DbError> {
    let (contains, prefix) = patterns(query);
    let (libraries, max_age, block_unrated) = access.binds();
    let rows = sqlx::query_as::<_, CatalogRow>(
        "SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                NULL::int AS season, NULL::int AS episode, m.created_at
         FROM media_items m
         WHERE m.item_type = 'movie'
           AND (m.title ILIKE $1 OR lisek_search_text(m.metadata) ILIKE $1)
           AND lisek_visible(m.library_id, m.age_rating, $4, $5, $6)
         ORDER BY (m.title ILIKE $1) DESC, (m.title ILIKE $2) DESC,
                  similarity(m.title, $3) DESC, m.title, m.id
         LIMIT $7",
    )
    .bind(&contains)
    .bind(&prefix)
    .bind(query.trim())
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(into_items(rows))
}

/// Series matching the query by title, or by the original title, genres
/// or tags of their episodes (every episode carries the series' provider
/// data; the representative one stands for the series).
pub async fn series(
    pool: &PgPool,
    query: &str,
    access: &Restrictions,
    limit: i64,
) -> Result<Vec<MediaItem>, DbError> {
    let (contains, prefix) = patterns(query);
    let (libraries, max_age, block_unrated) = access.binds();
    let rows = sqlx::query_as::<_, CatalogRow>(
        "SELECT s.representative_item_id AS id, s.library_id, 'series' AS item_type, s.title,
                NULL::int AS year, r.overview, NULL::int AS season, NULL::int AS episode,
                s.last_episode_added_at AS created_at
         FROM series s
         JOIN media_items r ON r.id = s.representative_item_id
         WHERE (s.title ILIKE $1
                OR s.representative_item_id IN (
                    SELECT m.id FROM media_items m
                    WHERE m.item_type = 'episode' AND lisek_search_text(m.metadata) ILIKE $1))
           AND lisek_visible(s.library_id, r.age_rating, $4, $5, $6)
         ORDER BY (s.title ILIKE $1) DESC, (s.title ILIKE $2) DESC,
                  similarity(s.title, $3) DESC, s.title, s.id
         LIMIT $7",
    )
    .bind(&contains)
    .bind(&prefix)
    .bind(query.trim())
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(into_items(rows))
}

/// An episode found by its own title.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct EpisodeHit {
    pub id: Uuid,
    pub series_title: String,
    pub season: i32,
    pub episode: i32,
    pub title: String,
}

pub async fn episodes(
    pool: &PgPool,
    query: &str,
    access: &Restrictions,
    limit: i64,
) -> Result<Vec<EpisodeHit>, DbError> {
    let (contains, prefix) = patterns(query);
    let (libraries, max_age, block_unrated) = access.binds();
    Ok(sqlx::query_as::<_, EpisodeHit>(
        "SELECT m.id, m.title AS series_title, e.season_number AS season,
                e.episode_number AS episode,
                m.metadata->'provider_data'->'episode'->>'title' AS title
         FROM media_items m
         JOIN episodes e ON e.media_item_id = m.id
         WHERE m.item_type = 'episode'
           AND (m.metadata->'provider_data'->'episode'->>'title') ILIKE $1
           AND lisek_visible(m.library_id, m.age_rating, $4, $5, $6)
         ORDER BY ((m.metadata->'provider_data'->'episode'->>'title') ILIKE $2) DESC,
                  similarity(m.metadata->'provider_data'->'episode'->>'title', $3) DESC,
                  m.title, e.season_number, e.episode_number, m.id
         LIMIT $7",
    )
    .bind(&contains)
    .bind(&prefix)
    .bind(query.trim())
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// A person found by name, credited on at least one visible title.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PersonHit {
    pub id: Uuid,
    pub name: String,
    pub has_image: bool,
    /// What they are best known for here: a part (actors) or the role.
    pub role: String,
    pub character: Option<String>,
}

pub async fn people(
    pool: &PgPool,
    query: &str,
    access: &Restrictions,
    limit: i64,
) -> Result<Vec<PersonHit>, DbError> {
    let (contains, prefix) = patterns(query);
    let (libraries, max_age, block_unrated) = access.binds();
    Ok(sqlx::query_as::<_, PersonHit>(
        "SELECT p.id, p.name, p.profile_url IS NOT NULL AS has_image, best.role, best.character_name AS character
         FROM people p
         CROSS JOIN LATERAL (
             SELECT mp.role, mp.character_name, count(*) OVER () AS credits
             FROM media_people mp
             JOIN media_items m ON m.id = mp.media_item_id
             WHERE mp.person_id = p.id
               AND lisek_visible(m.library_id, m.age_rating, $4, $5, $6)
             ORDER BY (mp.role = 'Actor') DESC, mp.sort_order
             LIMIT 1
         ) best
         WHERE p.name ILIKE $1
         ORDER BY (p.name ILIKE $2) DESC, similarity(p.name, $3) DESC, best.credits DESC, p.name, p.id
         LIMIT $7",
    )
    .bind(&contains)
    .bind(&prefix)
    .bind(query.trim())
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// A collection found by name, with how many films the account can see.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CollectionHit {
    pub id: Uuid,
    pub name: String,
    pub item_count: i64,
    pub first_year: Option<i32>,
}

pub async fn collections(
    pool: &PgPool,
    query: &str,
    access: &Restrictions,
    limit: i64,
) -> Result<Vec<CollectionHit>, DbError> {
    let (contains, prefix) = patterns(query);
    let (libraries, max_age, block_unrated) = access.binds();
    Ok(sqlx::query_as::<_, CollectionHit>(
        "SELECT c.id, c.name, count(*) AS item_count, min(m.year) AS first_year
         FROM collections c
         JOIN collection_items ci ON ci.collection_id = c.id
         JOIN media_items m ON m.id = ci.media_item_id
         WHERE c.name ILIKE $1
           AND lisek_visible(m.library_id, m.age_rating, $4, $5, $6)
         GROUP BY c.id
         ORDER BY (c.name ILIKE $2) DESC, similarity(c.name, $3) DESC, c.name, c.id
         LIMIT $7",
    )
    .bind(&contains)
    .bind(&prefix)
    .bind(query.trim())
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_in_the_query_are_literal() {
        assert_eq!(
            patterns(" 50%_off "),
            ("%50\\%\\_off%".into(), "50\\%\\_off%".into())
        );
    }
}
