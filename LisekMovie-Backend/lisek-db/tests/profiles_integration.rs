//! Database-backed tests for Faza 19 profiles: online status, avatar
//! version, own UI language and the server's default language. Opt-in:
//! see `common/mod.rs`.

use std::path::PathBuf;

use lisek_db::profiles;
use lisek_library::{FileFacts, ScanResult, ScannedMovie};
use uuid::Uuid;

mod common;
use common::TestDb;

async fn user(pool: &lisek_db::PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO users (username, password_hash) VALUES ($1, 'x') RETURNING id")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn session(pool: &lisek_db::PgPool, user: Uuid, token: &str, seen_ago_seconds: i64) {
    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, expires_at, last_seen_at) \
         VALUES ($1, $2, now() + interval '1 day', now() - make_interval(secs => $3::double precision))",
    )
    .bind(user)
    .bind(token)
    .bind(seen_ago_seconds as f64)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn presence_comes_from_recent_requests_and_playback() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let active = user(pool, "active").await;
    let idle = user(pool, "idle").await;
    let watching = user(pool, "watching").await;
    user(pool, "never").await;
    session(pool, active, "a", 30).await;
    session(pool, idle, "i", 3600).await;
    session(pool, watching, "w", 3600).await;

    let library = lisek_db::create_library(pool, "Filmy", "movies", &["/media".to_string()], false)
        .await
        .unwrap()
        .id;
    let film = ScanResult::Movie(ScannedMovie {
        file_path: PathBuf::from("/media/Film (2020).mkv"),
        title: "Film".into(),
        year: Some(2020),
        file: FileFacts { size_bytes: Some(1), modified: None },
    });
    lisek_db::persist_scan_results(pool, library, &[film]).await.unwrap();
    let item: Uuid = sqlx::query_scalar("SELECT id FROM media_items").fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO playback_sessions (user_id, item_id, auth_token_hash, ticket_hash) VALUES ($1, $2, 'w', 't')")
        .bind(watching)
        .bind(item)
        .execute(pool)
        .await
        .unwrap();

    let list = profiles::list_with_presence(pool).await.unwrap();
    let by_name = |name: &str| list.iter().find(|u| u.user.username == name).unwrap();
    assert!(by_name("active").online);
    assert!(!by_name("idle").online);
    assert!(by_name("idle").last_seen_at.is_some());
    assert!(by_name("watching").online);
    assert_eq!(by_name("watching").now_playing.as_deref(), Some("Film"));
    assert!(!by_name("never").online);
    assert!(by_name("never").last_seen_at.is_none());

    // A disabled account is never online.
    sqlx::query("UPDATE users SET disabled = true WHERE id = $1").bind(active).execute(pool).await.unwrap();
    let list = profiles::list_with_presence(pool).await.unwrap();
    assert!(!list.iter().find(|u| u.user.id == active).unwrap().online);

    // Touching only writes once the previous mark is a minute old.
    profiles::touch_session(pool, "i").await.unwrap();
    let idle_marked: bool =
        sqlx::query_scalar("SELECT last_seen_at > now() - interval '5 seconds' FROM sessions WHERE token_hash = 'i'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(idle_marked);
    session(pool, idle, "fresh", 10).await;
    profiles::touch_session(pool, "fresh").await.unwrap();
    let fresh_untouched: bool =
        sqlx::query_scalar("SELECT last_seen_at < now() - interval '5 seconds' FROM sessions WHERE token_hash = 'fresh'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(fresh_untouched, "a mark under a minute old is left alone");
    db.drop().await;
}

#[tokio::test]
async fn avatar_language_and_server_default() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let id = user(pool, "profile").await;

    assert_eq!(profiles::avatar_version(pool, id).await.unwrap(), Some(None));
    assert_eq!(profiles::avatar_version(pool, Uuid::new_v4()).await.unwrap(), None);
    profiles::set_avatar_version(pool, id, Some(42)).await.unwrap();
    assert_eq!(profiles::avatar_version(pool, id).await.unwrap(), Some(Some(42)));
    profiles::set_avatar_version(pool, id, None).await.unwrap();
    assert_eq!(profiles::avatar_version(pool, id).await.unwrap(), Some(None));

    let updated = profiles::set_ui_language(pool, id, "pl").await.unwrap().unwrap();
    assert_eq!(updated.ui_language, "pl");

    // The migration seeds the row; the wizard overwrites it.
    assert_eq!(profiles::default_language(pool).await.unwrap(), "en");
    profiles::set_default_language(pool, "pl").await.unwrap();
    assert_eq!(profiles::default_language(pool).await.unwrap(), "pl");
    assert!(profiles::set_default_language(pool, "de").await.is_err());
    db.drop().await;
}
