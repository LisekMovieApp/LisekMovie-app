//! Collections ("box sets"): films a metadata provider groups into one
//! series, e.g. TMDB's "Star Wars Collection". Rows are created and linked
//! during metadata enrichment (`lisek-server::metadata_enrichment`); every
//! read here applies the account's [`Restrictions`], so a collection shows
//! only the films that account may see.

use std::collections::HashMap;

use serde::Serialize;
use uuid::Uuid;

use lisek_core::MediaItem;

use crate::access::Restrictions;
use crate::catalog::{CatalogRow, into_items};
use crate::{DbError, PgPool};

/// A collection as stored, with the source URLs of its downloaded artwork.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StoredCollection {
    pub id: Uuid,
    pub name: String,
    pub poster_url: Option<String>,
    pub backdrop_url: Option<String>,
}

/// Creates the collection `(provider, provider_id)` or renames the
/// existing one. Returns it with the artwork URLs downloaded so far.
pub async fn upsert(
    pool: &PgPool,
    provider: &str,
    provider_id: &str,
    name: &str,
) -> Result<StoredCollection, DbError> {
    Ok(sqlx::query_as::<_, StoredCollection>(
        "INSERT INTO collections (provider, provider_id, name) VALUES ($1, $2, $3)
         ON CONFLICT (provider, provider_id) DO UPDATE SET
             name = EXCLUDED.name,
             updated_at = CASE WHEN collections.name = EXCLUDED.name
                               THEN collections.updated_at ELSE now() END
         RETURNING id, name, poster_url, backdrop_url",
    )
    .bind(provider)
    .bind(provider_id)
    .bind(name)
    .fetch_one(pool)
    .await?)
}

/// Makes `item_id` a member of `collection_id` and of no other collection
/// from the same provider (a film belongs to one TMDB collection).
pub async fn link_item(pool: &PgPool, collection_id: Uuid, item_id: Uuid) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM collection_items ci USING collections c, collections target
         WHERE ci.media_item_id = $2 AND c.id = ci.collection_id
           AND target.id = $1 AND c.provider = target.provider AND c.id <> $1",
    )
    .bind(collection_id)
    .bind(item_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO collection_items (collection_id, media_item_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(collection_id)
    .bind(item_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Removes `item_id` from every collection of `provider` (the provider
/// says the film belongs to none).
pub async fn unlink_item(pool: &PgPool, item_id: Uuid, provider: &str) -> Result<(), DbError> {
    sqlx::query(
        "DELETE FROM collection_items ci USING collections c
         WHERE ci.media_item_id = $1 AND c.id = ci.collection_id AND c.provider = $2",
    )
    .bind(item_id)
    .bind(provider)
    .execute(pool)
    .await?;
    Ok(())
}

/// Records which artwork URLs are now on disk for a collection.
pub async fn set_artwork(
    pool: &PgPool,
    collection_id: Uuid,
    poster_url: Option<&str>,
    backdrop_url: Option<&str>,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE collections SET poster_url = $2, backdrop_url = $3, updated_at = now() WHERE id = $1",
    )
    .bind(collection_id)
    .bind(poster_url)
    .bind(backdrop_url)
    .execute(pool)
    .await?;
    Ok(())
}

/// Deletes collections without members; returns their ids so the caller
/// can remove their artwork.
pub async fn prune_empty(pool: &PgPool) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "DELETE FROM collections c
         WHERE NOT EXISTS (SELECT 1 FROM collection_items ci WHERE ci.collection_id = c.id)
         RETURNING c.id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Every collection id — the artwork sweep keeps only these folders.
pub async fn all_ids(pool: &PgPool) -> Result<Vec<Uuid>, DbError> {
    let rows: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM collections")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// One collection card: how many films the account can see and the span
/// of their release years.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct CollectionSummary {
    pub id: Uuid,
    pub name: String,
    pub item_count: i64,
    pub first_year: Option<i32>,
    pub last_year: Option<i32>,
}

/// Collections with at least two visible films in `library_id`, by name.
pub async fn library_collections(
    pool: &PgPool,
    library_id: Uuid,
    access: &Restrictions,
    limit: i64,
) -> Result<Vec<CollectionSummary>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    Ok(sqlx::query_as::<_, CollectionSummary>(
        "SELECT c.id, c.name, count(*) AS item_count, min(m.year) AS first_year,
                max(m.year) AS last_year
         FROM collections c
         JOIN collection_items ci ON ci.collection_id = c.id
         JOIN media_items m ON m.id = ci.media_item_id
         WHERE m.item_type = 'movie' AND m.library_id = $1
           AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
         GROUP BY c.id
         HAVING count(*) >= 2
         ORDER BY c.name, c.id
         LIMIT $5",
    )
    .bind(library_id)
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .bind(limit)
    .fetch_all(pool)
    .await?)
}

/// One collection across every library. `None` when it does not exist or
/// the account can see none of its films.
pub async fn get(
    pool: &PgPool,
    collection_id: Uuid,
    access: &Restrictions,
) -> Result<Option<CollectionSummary>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    Ok(sqlx::query_as::<_, CollectionSummary>(
        "SELECT c.id, c.name, count(*) AS item_count, min(m.year) AS first_year,
                max(m.year) AS last_year
         FROM collections c
         JOIN collection_items ci ON ci.collection_id = c.id
         JOIN media_items m ON m.id = ci.media_item_id
         WHERE c.id = $1 AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
         GROUP BY c.id",
    )
    .bind(collection_id)
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .fetch_optional(pool)
    .await?)
}

/// The visible films of a collection in release order (undated last).
pub async fn members(
    pool: &PgPool,
    collection_id: Uuid,
    access: &Restrictions,
) -> Result<Vec<MediaItem>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    let rows = sqlx::query_as::<_, CatalogRow>(
        "SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                NULL::int AS season, NULL::int AS episode, m.created_at
         FROM collection_items ci
         JOIN media_items m ON m.id = ci.media_item_id
         WHERE ci.collection_id = $1
           AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
         ORDER BY m.year ASC NULLS LAST, m.title, m.id",
    )
    .bind(collection_id)
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .fetch_all(pool)
    .await?;
    Ok(into_items(rows))
}

/// The collection a film belongs to (the oldest link when there are
/// several).
pub async fn of_item(pool: &PgPool, item_id: Uuid) -> Result<Option<Uuid>, DbError> {
    let row: Option<(Uuid,)> = sqlx::query_as(
        "SELECT ci.collection_id FROM collection_items ci
         JOIN collections c ON c.id = ci.collection_id
         WHERE ci.media_item_id = $1
         ORDER BY c.created_at, c.id LIMIT 1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(id,)| id))
}

/// Visible films per collection inside one library, for collection cards
/// on a library page.
pub async fn counts_in_library(
    pool: &PgPool,
    collection_ids: &[Uuid],
    library_id: Uuid,
    access: &Restrictions,
) -> Result<HashMap<Uuid, i64>, DbError> {
    if collection_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let (libraries, max_age, block_unrated) = access.binds();
    let rows: Vec<(Uuid, i64)> = sqlx::query_as(
        "SELECT ci.collection_id, count(*)
         FROM collection_items ci JOIN media_items m ON m.id = ci.media_item_id
         WHERE ci.collection_id = ANY($1) AND m.library_id = $2 AND m.item_type = 'movie'
           AND lisek_visible(m.library_id, m.age_rating, $3, $4, $5)
         GROUP BY ci.collection_id",
    )
    .bind(collection_ids)
    .bind(library_id)
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// The visible film whose own artwork stands in for a collection without
/// any: the first one in release order.
pub async fn artwork_stand_in(
    pool: &PgPool,
    collection_id: Uuid,
    access: &Restrictions,
) -> Result<Option<Uuid>, DbError> {
    Ok(members(pool, collection_id, access)
        .await?
        .into_iter()
        .next()
        .map(|item| item.id))
}
