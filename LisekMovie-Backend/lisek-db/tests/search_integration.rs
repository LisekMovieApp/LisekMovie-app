//! Database-backed tests for the global search (Faza 11).
//!
//! Opt-in: see `common/mod.rs`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lisek_db::PgPool;
use lisek_db::access::Restrictions;
use lisek_db::people::NewCredit;
use lisek_db::search;
use lisek_library::{FileFacts, ScanResult, ScannedEpisode, ScannedMovie};
use uuid::Uuid;

mod common;
use common::TestDb;

fn facts() -> FileFacts {
    FileFacts {
        size_bytes: Some(1),
        modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
    }
}

async fn id_of(pool: &PgPool, file: &str) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as("SELECT id FROM media_items WHERE file_path = $1")
        .bind(file)
        .fetch_one(pool)
        .await
        .unwrap();
    id
}

async fn provider_data(pool: &PgPool, id: Uuid, data: serde_json::Value) {
    lisek_db::attach_provider_metadata(pool, id, "tmdb", &data)
        .await
        .unwrap();
}

#[tokio::test]
async fn search_finds_every_kind_and_respects_restrictions() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let films = lisek_db::create_library(pool, "Filmy", "movies", &["/m".into()], false)
        .await
        .unwrap()
        .id;
    let shows = lisek_db::create_library(pool, "Seriale", "tv_shows", &["/s".into()], false)
        .await
        .unwrap()
        .id;
    let movies: Vec<ScanResult> = [
        ("Za szybcy, za wściekli", 2003),
        ("Szybki strzał", 1999),
        ("Ocean's Eleven", 2001),
    ]
    .into_iter()
    .map(|(title, year)| {
        ScanResult::Movie(ScannedMovie {
            file_path: PathBuf::from(format!("/m/{title}.mkv")),
            title: title.into(),
            year: Some(year),
            file: facts(),
        })
    })
    .collect();
    lisek_db::persist_scan_results(pool, films, &movies)
        .await
        .unwrap();
    let fast = id_of(pool, "/m/Za szybcy, za wściekli.mkv").await;
    let ocean = id_of(pool, "/m/Ocean's Eleven.mkv").await;
    provider_data(pool, fast, serde_json::json!({"original_title": "2 Fast 2 Furious", "genres": ["Akcja"], "tags": ["street race"]})).await;
    let episodes: Vec<ScanResult> = (1..=2)
        .map(|n| {
            ScanResult::Episode(ScannedEpisode {
                file_path: PathBuf::from(format!("/s/13 Posterunek/Season 01/S01E0{n}.mkv")),
                series_title: "13 Posterunek".into(),
                season: 1,
                episode: n,
                end_episode: None,
                file: facts(),
            })
        })
        .collect();
    lisek_db::persist_scan_results(pool, shows, &episodes)
        .await
        .unwrap();
    let e1 = id_of(pool, "/s/13 Posterunek/Season 01/S01E01.mkv").await;
    let e2 = id_of(pool, "/s/13 Posterunek/Season 01/S01E02.mkv").await;
    for (id, title) in [(e1, "Najwyższy czas"), (e2, "Szybka akcja")] {
        provider_data(
            pool,
            id,
            serde_json::json!({"genres": ["Komedia"], "episode": {"title": title}}),
        )
        .await;
    }
    lisek_db::people::replace_credits(
        pool,
        fast,
        &[NewCredit {
            tmdb_id: 7,
            name: "Paul Walker".into(),
            role: "Actor".into(),
            character: Some("Brian".into()),
            order: 0,
        }],
    )
    .await
    .unwrap();
    let saga = lisek_db::collections::upsert(pool, "tmdb", "9485", "Szybcy i wściekli - kolekcja")
        .await
        .unwrap();
    lisek_db::collections::link_item(pool, saga.id, fast)
        .await
        .unwrap();

    let all = Restrictions::NONE;
    let titles =
        |items: Vec<lisek_core::MediaItem>| items.into_iter().map(|i| i.title).collect::<Vec<_>>();
    // A title match ranks before a genre/tag match; ILIKE ignores case.
    assert_eq!(
        titles(search::movies(pool, "SZYB", &all, 10).await.unwrap()),
        ["Szybki strzał", "Za szybcy, za wściekli"]
    );
    assert_eq!(
        titles(search::movies(pool, "fast 2", &all, 10).await.unwrap()),
        ["Za szybcy, za wściekli"],
        "original title"
    );
    assert_eq!(
        titles(search::movies(pool, "street", &all, 10).await.unwrap()),
        ["Za szybcy, za wściekli"],
        "tag"
    );
    assert_eq!(
        titles(search::movies(pool, "Ocean's", &all, 10).await.unwrap()),
        ["Ocean's Eleven"]
    );
    assert!(
        search::movies(pool, "%", &all, 10)
            .await
            .unwrap()
            .is_empty(),
        "wildcards are literal"
    );
    let _ = ocean;

    let series = search::series(pool, "posterun", &all, 10).await.unwrap();
    assert_eq!((series.len(), series[0].item_type.as_str()), (1, "series"));
    assert_eq!(
        search::series(pool, "komedia", &all, 10)
            .await
            .unwrap()
            .len(),
        1,
        "by the episodes' genres"
    );

    let found = search::episodes(pool, "szybka", &all, 10).await.unwrap();
    assert_eq!(
        found
            .iter()
            .map(|e| (
                e.series_title.as_str(),
                e.season,
                e.episode,
                e.title.as_str()
            ))
            .collect::<Vec<_>>(),
        [("13 Posterunek", 1, 2, "Szybka akcja")]
    );
    let people = search::people(pool, "walker", &all, 10).await.unwrap();
    assert_eq!(
        (people[0].name.as_str(), people[0].character.as_deref()),
        ("Paul Walker", Some("Brian"))
    );
    let collections = search::collections(pool, "wściekli", &all, 10)
        .await
        .unwrap();
    assert_eq!((collections[0].id, collections[0].item_count), (saga.id, 1));

    // Only the series library: films, their people and collections are gone.
    let shows_only = Restrictions {
        libraries: Some(vec![shows]),
        ..Restrictions::NONE
    };
    assert!(
        search::movies(pool, "szyb", &shows_only, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        search::people(pool, "walker", &shows_only, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        search::collections(pool, "wściekli", &shows_only, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        search::episodes(pool, "szybka", &shows_only, 10)
            .await
            .unwrap()
            .len(),
        1
    );

    // The trigram indexes answer these queries at a real library's size
    // (5000 more films, 2000 people, statistics refreshed). Small tables like
    // `people` are read whole by the planner's choice (a few ms); their
    // trigram index takes over as they grow.
    sqlx::query(
        "INSERT INTO media_items (library_id, item_type, title, file_path, metadata)
         SELECT $1, 'movie', 'Film ' || md5(n::text), '/m/gen/' || n || '.mkv',
                jsonb_build_object('provider_data', jsonb_build_object(
                    'genres', jsonb_build_array('Dramat'), 'tags', jsonb_build_array(md5(n::text))))
         FROM generate_series(1, 5000) AS n",
    )
    .bind(films)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO people (tmdb_id, name) SELECT 1000 + n, 'Person ' || md5(n::text) FROM generate_series(1, 2000) AS n")
        .execute(pool)
        .await
        .unwrap();
    for table in ["media_items", "people", "collections", "series"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("ANALYZE {table}")))
            .execute(pool)
            .await
            .unwrap();
    }
    let mut tx = pool.begin().await.unwrap();
    for (sql, indexes) in [
        (
            "EXPLAIN SELECT id FROM media_items WHERE item_type = 'movie' AND (title ILIKE '%szyb%' OR lisek_search_text(metadata) ILIKE '%szyb%')",
            &["media_items_title_trgm", "media_items_search_text_trgm"][..],
        ),
        (
            "EXPLAIN SELECT id FROM media_items WHERE item_type = 'episode' AND (metadata->'provider_data'->'episode'->>'title') ILIKE '%szyb%'",
            &["media_items_episode_title_trgm"][..],
        ),
    ] {
        let plan: Vec<(String,)> = sqlx::query_as(sql).fetch_all(&mut *tx).await.unwrap();
        let plan = plan
            .into_iter()
            .map(|(l,)| l)
            .collect::<Vec<_>>()
            .join("\n");
        for index in indexes {
            assert!(plan.contains(index), "{sql}\n{plan}");
        }
    }
    assert_eq!(
        titles(search::movies(pool, "szyb", &all, 10).await.unwrap()).len(),
        2,
        "same answer at scale"
    );
    tx.rollback().await.unwrap();

    db.drop().await;
}
