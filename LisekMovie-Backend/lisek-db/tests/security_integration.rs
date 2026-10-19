//! Database-backed tests for migration checksum healing and provider key
//! sealing. Opt-in: see `common/mod.rs`.

mod common;
use common::TestDb;

use lisek_db::StoredApiKey;

/// Same migration file, other line endings.
fn other_line_endings(sql: &str) -> String {
    if sql.contains("\r\n") {
        sql.replace("\r\n", "\n")
    } else {
        sql.replace('\n', "\r\n")
    }
}

#[tokio::test]
async fn migrations_tolerate_a_line_ending_change_but_not_an_edit() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let file = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/migrations/0001_initial.sql"
    ))
    .unwrap();
    let set_checksum = |content: String| {
        let pool = db.pool.clone();
        async move {
            sqlx::query("UPDATE _sqlx_migrations SET checksum = sha384(convert_to($1, 'UTF8')) WHERE version = 1")
                .bind(content)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    let stored = || async {
        sqlx::query_scalar::<_, Vec<u8>>("SELECT checksum FROM _sqlx_migrations WHERE version = 1")
            .fetch_one(&db.pool)
            .await
            .unwrap()
    };
    let original = stored().await;

    // As if the database was migrated from a checkout with other line endings.
    set_checksum(other_line_endings(&file)).await;
    assert_ne!(stored().await, original);
    lisek_db::run_migrations(&db.reconnect().await)
        .await
        .unwrap();
    assert_eq!(stored().await, original, "checksum healed");

    // A real edit is still refused.
    set_checksum(format!("-- edited\n{file}")).await;
    let err = lisek_db::run_migrations(&db.reconnect().await)
        .await
        .unwrap_err();
    assert!(
        matches!(err, lisek_db::DbError::MigrationFailed(_)),
        "{err}"
    );
    db.drop().await;
}

#[tokio::test]
async fn legacy_plaintext_provider_keys_are_sealed_once() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    sqlx::query("UPDATE provider_config SET settings = '{\"api_key\": \"legacy-key\"}' WHERE provider_key = 'tmdb'")
        .execute(pool)
        .await
        .unwrap();
    let keys = lisek_db::list_enabled_provider_keys(pool).await.unwrap();
    assert_eq!(
        keys,
        vec![("tmdb".to_string(), StoredApiKey::Plain("legacy-key".into()))]
    );

    let seal = |provider: &str, plain: &str| format!("v1:sealed({provider},{})", plain.len());
    assert_eq!(
        lisek_db::seal_legacy_provider_keys(pool, seal)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        lisek_db::seal_legacy_provider_keys(pool, seal)
            .await
            .unwrap(),
        0,
        "idempotent"
    );

    let settings: String = sqlx::query_scalar(
        "SELECT settings::text FROM provider_config WHERE provider_key = 'tmdb'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(!settings.contains("legacy-key"), "{settings}");
    let keys = lisek_db::list_enabled_provider_keys(pool).await.unwrap();
    assert_eq!(
        keys,
        vec![(
            "tmdb".to_string(),
            StoredApiKey::Sealed("v1:sealed(tmdb,10)".into())
        )]
    );
    let configured = lisek_db::list_provider_configs(pool).await.unwrap();
    assert!(
        configured
            .iter()
            .any(|p| p.provider_key == "tmdb" && p.is_configured)
    );

    lisek_db::clear_provider_api_key(pool, "tmdb")
        .await
        .unwrap();
    assert!(
        lisek_db::list_enabled_provider_keys(pool)
            .await
            .unwrap()
            .is_empty()
    );
    db.drop().await;
}
