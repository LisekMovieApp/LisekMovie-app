//! Database-backed tests for the series pages (Faza 10): season cards,
//! season pages with the viewer's state, season flags and metadata.
//!
//! Opt-in: see `common/mod.rs`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lisek_db::PgPool;
use lisek_db::seasons;
use lisek_library::{FileFacts, ScanResult, ScannedEpisode};
use uuid::Uuid;

mod common;
use common::TestDb;

fn facts() -> FileFacts {
    FileFacts {
        size_bytes: Some(1),
        modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
    }
}

fn path(season: i32, episode: i32) -> String {
    format!("/media/Seriale/BB/Season {season:02}/S{season:02}E{episode:02}.mkv")
}

async fn id_of(pool: &PgPool, file: &str) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as("SELECT id FROM media_items WHERE file_path = $1")
        .bind(file)
        .fetch_one(pool)
        .await
        .unwrap();
    id
}

async fn user(pool: &PgPool, name: &str) -> Uuid {
    let (id,): (Uuid,) =
        sqlx::query_as("INSERT INTO users (username, password_hash) VALUES ($1, 'x') RETURNING id")
            .bind(name)
            .fetch_one(pool)
            .await
            .unwrap();
    id
}

#[tokio::test]
async fn seasons_cards_pages_and_flags() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let tv = lisek_db::create_library(
        pool,
        "Seriale",
        "tv_shows",
        &["/media/Seriale".into()],
        false,
    )
    .await
    .unwrap()
    .id;
    let files: Vec<ScanResult> = [(1, 1), (1, 2), (1, 3), (2, 1), (0, 1)]
        .into_iter()
        .map(|(season, episode)| {
            ScanResult::Episode(ScannedEpisode {
                file_path: PathBuf::from(path(season, episode)),
                series_title: "Breaking Bad".into(),
                season,
                episode,
                end_episode: None,
                file: facts(),
            })
        })
        .collect();
    lisek_db::persist_scan_results(pool, tv, &files)
        .await
        .unwrap();
    let e1 = id_of(pool, &path(1, 1)).await;
    let e2 = id_of(pool, &path(1, 2)).await;
    let e3 = id_of(pool, &path(1, 3)).await;
    let s2 = id_of(pool, &path(2, 1)).await;
    let viewer = user(pool, "viewer").await;
    let other = user(pool, "other").await;

    let place = seasons::placement(pool, e2).await.unwrap().unwrap();
    assert_eq!((place.season, place.episode), (1, 2));
    let (stored, episode_files) = seasons::series_layout(pool, place.series_id).await.unwrap();
    assert_eq!(
        stored.iter().map(|s| s.number).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(episode_files.len(), 5);

    // Season metadata and a poster URL.
    seasons::set_season_metadata(
        pool,
        place.series_id,
        1,
        Some("Sezon 1"),
        Some("Walter..."),
        Some(2008),
    )
    .await
    .unwrap();
    seasons::set_season_poster_url(pool, place.series_id, 1, Some("https://x/s1.jpg"))
        .await
        .unwrap();

    // Episode data in provider_data survives a match without it.
    lisek_db::attach_provider_metadata(
        pool,
        e1,
        "tmdb",
        &serde_json::json!({"tmdb_id": 1396, "episode": {"title": "Pilot", "runtime_minutes": 58}}),
    )
    .await
    .unwrap();
    lisek_db::attach_provider_metadata(pool, e1, "tmdb", &serde_json::json!({"tmdb_id": 1396}))
        .await
        .unwrap();
    assert_eq!(
        seasons::episode_still_url(pool, e1).await.unwrap(),
        None,
        "no still named"
    );

    // Cards: specials last, play from the first unwatched episode.
    sqlx::query("UPDATE media_items SET duration_seconds = 3061 WHERE id = $1")
        .bind(e1)
        .execute(pool)
        .await
        .unwrap();
    lisek_db::playback::set_flags(pool, viewer, e1, Some(true), None)
        .await
        .unwrap();
    let cards = seasons::season_cards(pool, e3, viewer).await.unwrap();
    assert_eq!(
        cards.iter().map(|c| c.season).collect::<Vec<_>>(),
        [1, 2, 0]
    );
    let first = &cards[0];
    assert_eq!(
        (
            first.name.as_deref(),
            first.episode_count,
            first.unwatched_count,
            first.play_id,
            first.has_poster
        ),
        (Some("Sezon 1"), 3, 2, e2, true)
    );
    assert_eq!(cards[1].play_id, s2);
    assert!(!first.favorite);
    assert_eq!(
        seasons::season_cards(pool, e3, other).await.unwrap()[0].play_id,
        e1,
        "per viewer"
    );

    // The season page with this viewer's state.
    let page = seasons::season_page(pool, s2, 1, viewer)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            page.name.as_deref(),
            page.overview.as_deref(),
            page.air_year
        ),
        (Some("Sezon 1"), Some("Walter..."), Some(2008))
    );
    assert_eq!(
        page.episodes.iter().map(|e| e.id).collect::<Vec<_>>(),
        [e1, e2, e3]
    );
    assert!(page.episodes[0].watched && !page.episodes[1].watched);
    assert_eq!(page.episodes[0].data.as_ref().unwrap()["title"], "Pilot");
    assert_eq!(page.episodes[0].duration_seconds, Some(3061.0));
    assert!(
        seasons::season_page(pool, e1, 7, viewer)
            .await
            .unwrap()
            .is_none()
    );

    // Whole season: favourite, then watched (which clears resume points).
    assert_eq!(
        seasons::set_season_flags(pool, viewer, e1, 1, None, Some(true))
            .await
            .unwrap(),
        3
    );
    assert!(seasons::season_cards(pool, e1, viewer).await.unwrap()[0].favorite);
    sqlx::query(
        "UPDATE user_media_state SET position_seconds = 120 WHERE item_id = $1 AND user_id = $2",
    )
    .bind(e2)
    .bind(viewer)
    .execute(pool)
    .await
    .unwrap();
    seasons::set_season_flags(pool, viewer, e1, 1, Some(true), None)
        .await
        .unwrap();
    let page = seasons::season_page(pool, e1, 1, viewer)
        .await
        .unwrap()
        .unwrap();
    assert!(
        page.episodes
            .iter()
            .all(|e| e.watched && e.favorite && e.position_seconds == 0.0)
    );
    let card = &seasons::season_cards(pool, e1, viewer).await.unwrap()[0];
    assert_eq!(
        (card.unwatched_count, card.play_id),
        (0, e1),
        "all watched: from the start"
    );
    assert_eq!(
        seasons::set_season_flags(pool, viewer, e1, 9, Some(true), None)
            .await
            .unwrap(),
        0
    );

    let (series_id, file) = seasons::season_poster_source(pool, e1, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((series_id, file), (place.series_id, path(2, 1)));

    db.drop().await;
}
