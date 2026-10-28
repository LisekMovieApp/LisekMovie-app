//! Faza 19: account profiles (avatar, own UI language), presence (online
//! status for the admin user list) and the server's default language.
use chrono::{DateTime, Utc};
use lisek_core::User;
use uuid::Uuid;

use crate::{DbError, PgPool, UserRow, user_from_row};

/// An account counts as online when one of its sessions made a request
/// within this many seconds (the web app sends a heartbeat every minute
/// while a tab is visible), or while it is playing something.
pub const ONLINE_WINDOW_SECONDS: i64 = 180;

/// The server's default language (`server_settings`, set by the wizard).
/// `"en"` when the row is missing.
pub async fn default_language(pool: &PgPool) -> Result<String, DbError> {
    let language: Option<String> = sqlx::query_scalar("SELECT default_language FROM server_settings WHERE id")
        .fetch_optional(pool)
        .await?;
    Ok(language.unwrap_or_else(|| "en".to_owned()))
}

/// Stores the server's default language (the First-Run Wizard's choice).
pub async fn set_default_language(pool: &PgPool, language: &str) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO server_settings (id, default_language) VALUES (true, $1) \
         ON CONFLICT (id) DO UPDATE SET default_language = EXCLUDED.default_language",
    )
    .bind(language)
    .execute(pool)
    .await?;
    Ok(())
}

/// The account's avatar version: outer `None` when the account does not
/// exist, inner `None` when it has no avatar.
pub async fn avatar_version(pool: &PgPool, user_id: Uuid) -> Result<Option<Option<i64>>, DbError> {
    Ok(sqlx::query_scalar("SELECT avatar_version FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await?)
}

/// Sets (`Some`) or clears (`None`) the account's avatar version.
pub async fn set_avatar_version(pool: &PgPool, user_id: Uuid, version: Option<i64>) -> Result<(), DbError> {
    sqlx::query("UPDATE users SET avatar_version = $2, updated_at = now() WHERE id = $1")
        .bind(user_id)
        .bind(version)
        .execute(pool)
        .await?;
    Ok(())
}

/// The account's own UI language.
pub async fn set_ui_language(pool: &PgPool, user_id: Uuid, language: &str) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as::<_, UserRow>(
        "UPDATE users SET ui_language = $2, updated_at = now() WHERE id = $1 AND NOT disabled \
         RETURNING id,username,is_admin,ui_language,email,created_at,must_change_password,disabled",
    )
    .bind(user_id)
    .bind(language)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(user_from_row))
}

/// Marks a session as seen now. Skips the write when it was already
/// marked within the last minute.
pub async fn touch_session(pool: &PgPool, token_hash: &str) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE sessions SET last_seen_at = now() \
         WHERE token_hash = $1 AND last_seen_at < now() - interval '60 seconds'",
    )
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(())
}

/// One account in the admin user list, with its presence.
pub struct UserPresence {
    pub user: User,
    pub avatar_version: Option<i64>,
    /// The newest request of any of the account's sessions.
    pub last_seen_at: Option<DateTime<Utc>>,
    pub online: bool,
    /// Title of what the account is playing right now.
    pub now_playing: Option<String>,
}

type PresenceRow = (
    Uuid,
    String,
    bool,
    String,
    Option<String>,
    DateTime<Utc>,
    bool,
    bool,
    Option<i64>,
    Option<DateTime<Utc>>,
    bool,
    Option<String>,
);

/// Every account with avatar and presence, ordered like `users::list`.
pub async fn list_with_presence(pool: &PgPool) -> Result<Vec<UserPresence>, DbError> {
    let rows = sqlx::query_as::<_, PresenceRow>(
        "SELECT u.id, u.username, u.is_admin, u.ui_language, u.email, u.created_at, u.must_change_password, u.disabled, \
                u.avatar_version, seen.last_seen_at, \
                COALESCE(NOT u.disabled AND (play.title IS NOT NULL \
                    OR seen.last_seen_at > now() - make_interval(secs => $1::double precision)), false) AS online, \
                play.title \
         FROM users u \
         LEFT JOIN LATERAL ( \
             SELECT max(s.last_seen_at) AS last_seen_at FROM sessions s \
             WHERE s.user_id = u.id AND s.expires_at > now() \
         ) seen ON true \
         LEFT JOIN LATERAL ( \
             SELECT m.title FROM playback_sessions p JOIN media_items m ON m.id = p.item_id \
             WHERE p.user_id = u.id AND p.ended_at IS NULL \
               AND p.last_heartbeat_at > now() - interval '2 minutes' \
             ORDER BY p.last_heartbeat_at DESC LIMIT 1 \
         ) play ON true \
         ORDER BY lower(u.username), u.id",
    )
    .bind(ONLINE_WINDOW_SECONDS as f64)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| UserPresence {
            user: user_from_row((r.0, r.1, r.2, r.3, r.4, r.5, r.6, r.7)),
            avatar_version: r.8,
            last_seen_at: r.9,
            online: r.10,
            now_playing: r.11,
        })
        .collect())
}
