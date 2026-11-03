//! Faza 25: named API keys (`api_keys`). A key is a random token shown
//! once; only its hash is stored, so a lookup is by hash. `last_used_at`
//! tells the administrator whether a key is still in use; it is written at
//! most once a minute per key, not on every request.
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::{DbError, PgPool};

/// One key as the administrator sees it (never the key itself).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow, utoipa::ToSchema)]
pub struct ApiKey {
    pub id: Uuid,
    pub name: String,
    /// The key's first characters (`lsk_AbC1`), to tell keys apart.
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Every key, newest first.
pub async fn list(pool: &PgPool) -> Result<Vec<ApiKey>, DbError> {
    Ok(sqlx::query_as(
        "SELECT id, name, prefix, created_at, last_used_at FROM api_keys ORDER BY created_at DESC, name",
    )
    .fetch_all(pool)
    .await?)
}

/// Stores a new key (its hash) under `name`.
pub async fn create(
    pool: &PgPool,
    name: &str,
    token_hash: &str,
    prefix: &str,
    created_by: Option<Uuid>,
) -> Result<ApiKey, DbError> {
    Ok(sqlx::query_as(
        "INSERT INTO api_keys (name, token_hash, prefix, created_by) VALUES ($1, $2, $3, $4) \
         RETURNING id, name, prefix, created_at, last_used_at",
    )
    .bind(name)
    .bind(token_hash)
    .bind(prefix)
    .bind(created_by)
    .fetch_one(pool)
    .await?)
}

/// Renames a key. `false`: no such key.
pub async fn rename(pool: &PgPool, id: Uuid, name: &str) -> Result<bool, DbError> {
    Ok(sqlx::query("UPDATE api_keys SET name = $2 WHERE id = $1")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// Revokes a key. `false`: no such key.
pub async fn delete(pool: &PgPool, id: Uuid) -> Result<bool, DbError> {
    Ok(sqlx::query("DELETE FROM api_keys WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

/// The key with this hash, if any, marked as used now (at most once a
/// minute). Returns its id and name.
pub async fn authenticate(pool: &PgPool, token_hash: &str) -> Result<Option<(Uuid, String)>, DbError> {
    let found: Option<(Uuid, String, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT id, name, last_used_at FROM api_keys WHERE token_hash = $1")
            .bind(token_hash)
            .fetch_optional(pool)
            .await?;
    let Some((id, name, last_used)) = found else {
        return Ok(None);
    };
    if last_used.is_none_or(|at| Utc::now() - at > chrono::TimeDelta::seconds(60)) {
        sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(Some((id, name)))
}
