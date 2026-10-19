//! First admin creation and login sessions.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use lisek_core::User;

use crate::{DbError, PgPool};

/// The raw shape every `users` row comes back as — used both for the plain
/// `User` domain type and (with `password_hash` on the side) for login,
/// which needs the hash but must never let it leak into `User` itself.
pub(crate) type UserRow = (Uuid, String, bool, String, Option<String>, DateTime<Utc>, bool, bool);

pub(crate) fn user_from_row(row: UserRow) -> User {
    User {
        id: row.0,
        username: row.1,
        is_admin: row.2,
        ui_language: row.3,
        email: row.4,
        created_at: row.5,
        must_change_password: row.6,
        disabled: row.7,
    }
}

/// Creates the first admin account. Only ever called once, from the
/// wizard's "finish" step — nothing currently re-checks that no admin
/// already exists, because that guard lives at the setup-mode level in
/// lisek-server (setup endpoints refuse to run at all once a connection is
/// already configured).
pub async fn insert_admin_user(
    pool: &PgPool,
    username: &str,
    password_hash: &str,
    ui_language: &str,
) -> Result<User, DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(710019)").execute(&mut *tx).await?;
    let (exists,): (bool,) = sqlx::query_as("SELECT EXISTS(SELECT 1 FROM users WHERE is_admin)")
        .fetch_one(&mut *tx).await?;
    if exists { return Err(DbError::Other("Initial administrator already exists".into())); }
    let row = sqlx::query_as::<_, UserRow>(
        r#"
        INSERT INTO users (username, password_hash, is_admin, ui_language)
        VALUES ($1, $2, true, $3)
        RETURNING id, username, is_admin, ui_language, email, created_at, must_change_password, disabled
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(ui_language)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(user_from_row(row))
}

/// Looks up a user by username for login — returns the domain `User`
/// alongside the password hash as a separate value, deliberately never
/// attached to `User` itself (see [`lisek_core::User`]'s own doc comment
/// and its "never serializes a password" test). `None` means no such user;
/// the caller (lisek-server's login handler) is responsible for still
/// running a dummy password verification in that case, so a request for a
/// nonexistent username doesn't respond measurably faster than a wrong
/// password for a real one.
pub async fn find_user_with_password_hash(pool:&PgPool,username:&str)->Result<Option<(User,String)>,DbError>{
    let row:Option<(Uuid,String,bool,String,Option<String>,DateTime<Utc>,bool,bool,String)>=sqlx::query_as("SELECT id,username,is_admin,ui_language,email,created_at,must_change_password,disabled,password_hash FROM users WHERE username=$1")
        .bind(username).fetch_optional(pool).await?;
    Ok(row.map(|(id,name,admin,lang,email,created,change,disabled,hash)|(user_from_row((id,name,admin,lang,email,created,change,disabled)),hash)))
}

/// Records a new login session. `token_hash` must already be hashed (see
/// `lisek_auth::hash_session_token`) — this function never sees, and the
/// `sessions` table never stores, the raw token.
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    pool: &PgPool,
    user_id: Uuid,
    token_hash: &str,
    client: &str,
    device: &str,
    device_id: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO sessions (user_id, token_hash, client, device, device_id, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(client)
    .bind(device)
    .bind(device_id)
    .bind(expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// The per-request auth check: resolves an already-hashed session token to
/// the user it belongs to, or `None` if there is no such session, or it has
/// expired. This is the one query `AuthUser` (in lisek-server) runs on every
/// protected request.
pub async fn find_user_by_session_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<User>, DbError> {
    let row = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT u.id, u.username, u.is_admin, u.ui_language, u.email, u.created_at, u.must_change_password, u.disabled
        FROM sessions s
        JOIN users u ON u.id = s.user_id
        WHERE s.token_hash = $1 AND s.expires_at > now() AND NOT u.disabled
        "#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(user_from_row))
}

/// Deletes a session by its hashed token — logout. Deleting a token that
/// doesn't exist (already logged out, already expired) is not an error;
/// the end state either way is "this token no longer authenticates".
pub async fn delete_session_by_token_hash(pool: &PgPool, token_hash: &str) -> Result<(), DbError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(pool)
        .await?;

    Ok(())
}
