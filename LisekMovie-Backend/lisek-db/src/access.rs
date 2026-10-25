//! Per-account access restrictions (parental controls): which libraries an
//! account may browse and the highest age rating it may see.
//!
//! The rule itself lives in SQL (`lisek_visible`, migration 0018) so the
//! catalog queries filter inside their index scans. Every catalog function
//! takes a [`Restrictions`] and binds its three values; an unrestricted
//! account binds `NULL, NULL, false`, which the planner folds away.
//!
//! Administrators are never restricted: [`find_session`] hands them
//! [`Restrictions::NONE`] whatever their row says.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use lisek_core::User;

use crate::{DbError, PgPool, UserRow, user_from_row};

/// Highest age a rating can carry (`user_restrictions.max_age_rating`).
pub const MAX_AGE_RATING: i32 = 21;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Restrictions {
    /// `None`: every library. `Some`: only these (an empty list hides the
    /// whole catalog).
    pub libraries: Option<Vec<Uuid>>,
    /// Items rated above this minimum age are hidden. `None`: no limit.
    pub max_age_rating: Option<i32>,
    /// With an age limit, also hide items that have no rating at all.
    #[serde(default)]
    pub block_unrated: bool,
}

impl Restrictions {
    pub const NONE: Restrictions = Restrictions {
        libraries: None,
        max_age_rating: None,
        block_unrated: false,
    };

    pub fn is_unrestricted(&self) -> bool {
        self.libraries.is_none() && self.max_age_rating.is_none()
    }

    pub fn allows_library(&self, library_id: Uuid) -> bool {
        self.libraries
            .as_ref()
            .is_none_or(|libraries| libraries.contains(&library_id))
    }

    /// The three values `lisek_visible(...)` takes after the item's own
    /// library and rating.
    pub(crate) fn binds(&self) -> (Option<&[Uuid]>, Option<i32>, bool) {
        (
            self.libraries.as_deref(),
            self.max_age_rating,
            self.max_age_rating.is_some() && self.block_unrated,
        )
    }

    /// Canonical form: sorted, de-duplicated libraries; `block_unrated`
    /// only together with an age limit; the age clamped to its range.
    pub fn normalized(mut self) -> Self {
        if let Some(libraries) = self.libraries.as_mut() {
            libraries.sort();
            libraries.dedup();
        }
        self.max_age_rating = self.max_age_rating.map(|age| age.clamp(0, MAX_AGE_RATING));
        self.block_unrated &= self.max_age_rating.is_some();
        self
    }
}

type RestrictionsRow = (Option<Vec<Uuid>>, Option<i32>, bool);
/// `(is_admin, libraries, max_age_rating, block_unrated)`, the last three
/// NULL without a row.
type EditorRow = (bool, Option<Vec<Uuid>>, Option<i32>, Option<bool>);
type ListRow = (Uuid, Option<Vec<Uuid>>, Option<i32>, bool);

fn restrictions_from_row(row: Option<RestrictionsRow>) -> Restrictions {
    row.map(|(libraries, max_age_rating, block_unrated)| Restrictions {
        libraries,
        max_age_rating,
        block_unrated,
    })
    .unwrap_or_default()
    .normalized()
}

/// The stored restrictions of one account and whether it is an
/// administrator (the admin editor's view: an administrator's row is
/// returned as stored, though it is not enforced). `None` when the account
/// does not exist.
pub async fn get_restrictions(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<(bool, Restrictions)>, DbError> {
    let row: Option<EditorRow> = sqlx::query_as(
        "SELECT u.is_admin, r.libraries, r.max_age_rating, r.block_unrated
         FROM users u LEFT JOIN user_restrictions r ON r.user_id = u.id
         WHERE u.id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(is_admin, libraries, max_age_rating, block_unrated)| {
        (
            is_admin,
            restrictions_from_row(Some((libraries, max_age_rating, block_unrated.unwrap_or(false)))),
        )
    }))
}

/// Replaces an account's restrictions; no restrictions removes the row.
/// Returns `false` when the account does not exist.
pub async fn set_restrictions(
    pool: &PgPool,
    user_id: Uuid,
    restrictions: &Restrictions,
) -> Result<bool, DbError> {
    let restrictions = restrictions.clone().normalized();
    let exists: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Ok(false);
    }
    if restrictions.is_unrestricted() {
        sqlx::query("DELETE FROM user_restrictions WHERE user_id = $1")
            .bind(user_id)
            .execute(pool)
            .await?;
        return Ok(true);
    }
    sqlx::query(
        "INSERT INTO user_restrictions (user_id, libraries, max_age_rating, block_unrated)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id) DO UPDATE SET
             libraries = EXCLUDED.libraries,
             max_age_rating = EXCLUDED.max_age_rating,
             block_unrated = EXCLUDED.block_unrated,
             updated_at = now()",
    )
    .bind(user_id)
    .bind(restrictions.libraries.as_deref())
    .bind(restrictions.max_age_rating)
    .bind(restrictions.block_unrated)
    .execute(pool)
    .await?;
    Ok(true)
}

/// Every non-administrator account that browses with restrictions, with
/// them — the admin user list marks these accounts.
pub async fn list_restrictions(pool: &PgPool) -> Result<Vec<(Uuid, Restrictions)>, DbError> {
    let rows: Vec<ListRow> = sqlx::query_as(
        "SELECT r.user_id, r.libraries, r.max_age_rating, r.block_unrated
         FROM user_restrictions r JOIN users u ON u.id = r.user_id
         WHERE NOT u.is_admin AND (r.libraries IS NOT NULL OR r.max_age_rating IS NOT NULL)
         ORDER BY u.username",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, libraries, max_age_rating, block_unrated)| {
            (id, restrictions_from_row(Some((libraries, max_age_rating, block_unrated))))
        })
        .collect())
}

/// The per-request session lookup: the account behind a hashed session
/// token and the restrictions it browses with. `None` for an unknown or
/// expired session, or a disabled account.
pub async fn find_session(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<(User, Restrictions)>, DbError> {
    #[derive(sqlx::FromRow)]
    struct Row {
        id: Uuid,
        username: String,
        is_admin: bool,
        ui_language: String,
        email: Option<String>,
        created_at: chrono::DateTime<chrono::Utc>,
        must_change_password: bool,
        disabled: bool,
        libraries: Option<Vec<Uuid>>,
        max_age_rating: Option<i32>,
        block_unrated: Option<bool>,
    }
    let row = sqlx::query_as::<_, Row>(
        r#"
        SELECT u.id, u.username, u.is_admin, u.ui_language, u.email, u.created_at,
               u.must_change_password, u.disabled,
               r.libraries, r.max_age_rating, r.block_unrated
        FROM sessions s
        JOIN users u ON u.id = s.user_id
        LEFT JOIN user_restrictions r ON r.user_id = u.id
        WHERE s.token_hash = $1 AND s.expires_at > now() AND NOT u.disabled
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| {
        let user_row: UserRow = (
            row.id,
            row.username,
            row.is_admin,
            row.ui_language,
            row.email,
            row.created_at,
            row.must_change_password,
            row.disabled,
        );
        let restrictions = if row.is_admin {
            Restrictions::NONE
        } else {
            restrictions_from_row(Some((
                row.libraries,
                row.max_age_rating,
                row.block_unrated.unwrap_or(false),
            )))
        };
        (user_from_row(user_row), restrictions)
    }))
}

/// Whether `item_id` (a movie or an episode) may be shown under
/// `restrictions`. `None`: no such item. An unrestricted account still
/// gets the existence check.
pub async fn item_visible(
    pool: &PgPool,
    item_id: Uuid,
    restrictions: &Restrictions,
) -> Result<Option<bool>, DbError> {
    let (libraries, max_age, block_unrated) = restrictions.binds();
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT lisek_visible(library_id, age_rating, $2, $3, $4) FROM media_items WHERE id = $1",
    )
    .bind(item_id)
    .bind(libraries)
    .bind(max_age)
    .bind(block_unrated)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(visible,)| visible))
}

/// An item's age rating and the certification it came from (`None` for
/// a rating an administrator set by hand). Outer `None`: no such item.
pub async fn item_rating(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<(Option<i32>, Option<String>)>, DbError> {
    Ok(sqlx::query_as(
        "SELECT age_rating,
                CASE WHEN jsonb_typeof(metadata->'age_rating') = 'number' THEN NULL
                     ELSE metadata->'provider_data'->>'certification' END
         FROM media_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_drops_block_unrated_without_an_age_limit() {
        let r = Restrictions {
            libraries: None,
            max_age_rating: None,
            block_unrated: true,
        }
        .normalized();
        assert!(!r.block_unrated);
        assert!(r.is_unrestricted());
    }

    #[test]
    fn normalized_sorts_libraries_and_clamps_the_age() {
        let a = Uuid::from_u128(2);
        let b = Uuid::from_u128(1);
        let r = Restrictions {
            libraries: Some(vec![a, b, a]),
            max_age_rating: Some(99),
            block_unrated: true,
        }
        .normalized();
        assert_eq!(r.libraries, Some(vec![b, a]));
        assert_eq!(r.max_age_rating, Some(MAX_AGE_RATING));
        assert!(r.block_unrated);
        assert!(r.allows_library(a));
        assert!(!r.allows_library(Uuid::from_u128(3)));
    }

    #[test]
    fn an_empty_library_list_is_a_restriction() {
        let r = Restrictions {
            libraries: Some(vec![]),
            ..Restrictions::NONE
        };
        assert!(!r.is_unrestricted());
        assert!(!r.allows_library(Uuid::from_u128(1)));
        assert!(Restrictions::NONE.allows_library(Uuid::from_u128(1)));
    }
}
