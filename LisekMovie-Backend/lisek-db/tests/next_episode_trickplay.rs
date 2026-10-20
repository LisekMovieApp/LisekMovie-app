//! Database-backed tests for the "next episode" lookup and trickplay
//! bookkeeping. Opt-in: see `common/mod.rs`.

use std::path::PathBuf;

use lisek_db::episodes;
use lisek_db::trickplay::{self, TrickplayReady};
use lisek_library::{FileFacts, ScanResult, ScannedEpisode, ScannedMovie};
use uuid::Uuid;

mod common;
use common::TestDb;

fn episode(file: &str, season: i32, episode: i32, end_episode: Option<i32>) -> ScanResult {
    ScanResult::Episode(ScannedEpisode {
        file_path: PathBuf::from(format!("/tv/Lisek/{file}")),
        series_title: "Lisek".into(),
        season,
        episode,
        end_episode,
        file: FileFacts {
            size_bytes: Some(1),
            modified: None,
        },
    })
}

async fn id_of(pool: &lisek_db::PgPool, file: &str) -> Uuid {
    let (id,): (Uuid,) =
        sqlx::query_as("SELECT id FROM media_items WHERE file_path LIKE '%' || $1")
            .bind(file)
            .fetch_one(pool)
            .await
            .unwrap();
    id
}

#[tokio::test]
async fn next_episode_follows_season_and_episode_order() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let tv = lisek_db::create_library(pool, "Seriale", "tv_shows", &["/tv".to_string()], false)
        .await
        .unwrap()
        .id;
    let files = [
        episode("S01E01.mkv", 1, 1, None),
        episode("S01E02-E03.mkv", 1, 2, Some(3)),
        episode("S01E04.mkv", 1, 4, None),
        episode("S01E04.1080p.mkv", 1, 4, None),
        episode("S02E01.mkv", 2, 1, None),
        episode("S00E01.mkv", 0, 1, None),
    ];
    lisek_db::persist_scan_results(pool, tv, &files)
        .await
        .unwrap();
    let films = lisek_db::create_library(pool, "Filmy", "movies", &["/films".to_string()], false)
        .await
        .unwrap()
        .id;
    let movie = ScanResult::Movie(ScannedMovie {
        file_path: PathBuf::from("/films/Film (2020).mkv"),
        title: "Film".into(),
        year: Some(2020),
        file: FileFacts {
            size_bytes: Some(1),
            modified: None,
        },
    });
    lisek_db::persist_scan_results(pool, films, &[movie])
        .await
        .unwrap();

    let next = |file: &'static str| async move {
        let id = id_of(pool, file).await;
        episodes::next_episode(pool, id)
            .await
            .unwrap()
            .map(|n| (n.season, n.episode))
    };
    assert_eq!(next("/S01E01.mkv").await, Some((1, 2)));
    assert_eq!(
        next("/S01E02-E03.mkv").await,
        Some((1, 4)),
        "a double episode skips its second half"
    );
    assert_eq!(
        next("/S01E04.mkv").await,
        Some((2, 1)),
        "another copy of the same episode is skipped"
    );
    assert_eq!(next("/S02E01.mkv").await, None, "last episode");
    assert_eq!(
        next("/S00E01.mkv").await,
        Some((1, 1)),
        "after a special, the regular order"
    );
    assert_eq!(
        next("Film (2020).mkv").await,
        None,
        "movies have no next episode"
    );
    let named = episodes::next_episode(pool, id_of(pool, "/S01E01.mkv").await)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(named.title, "Lisek");
    db.drop().await;
}

#[tokio::test]
async fn trickplay_candidates_follow_file_changes_and_failures() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let films = lisek_db::create_library(pool, "Filmy", "movies", &["/films".to_string()], false)
        .await
        .unwrap()
        .id;
    let movie = ScanResult::Movie(ScannedMovie {
        file_path: PathBuf::from("/films/Film (2020).mkv"),
        title: "Film".into(),
        year: Some(2020),
        file: FileFacts {
            size_bytes: Some(10),
            modified: None,
        },
    });
    lisek_db::persist_scan_results(pool, films, &[movie])
        .await
        .unwrap();
    let id = id_of(pool, "Film (2020).mkv").await;
    assert!(
        trickplay::trickplay_candidates(pool, 10)
            .await
            .unwrap()
            .is_empty(),
        "not probed yet"
    );

    sqlx::query("UPDATE media_items SET probed_at = now(), duration_seconds = 5400, file_size = 10 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    // Cover art alone is not a video to preview.
    sqlx::query("INSERT INTO media_streams (media_item_id, stream_index, stream_type, codec, width, height) VALUES ($1, 3, 'video', 'mjpeg', 600, 900)")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert!(
        trickplay::trickplay_candidates(pool, 10)
            .await
            .unwrap()
            .is_empty()
    );
    sqlx::query("INSERT INTO media_streams (media_item_id, stream_index, stream_type, codec, width, height, color_transfer) VALUES ($1, 0, 'video', 'hevc', 3840, 2160, 'smpte2084')")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    let candidates = trickplay::trickplay_candidates(pool, 10).await.unwrap();
    assert_eq!(candidates.len(), 1);
    let c = &candidates[0];
    assert_eq!(
        (c.width, c.height, c.color_transfer.as_deref()),
        (Some(3840), Some(2160), Some("smpte2084"))
    );
    assert_eq!(trickplay::trickplay_stats(pool).await.unwrap().pending, 1);

    trickplay::save_trickplay_ready(
        pool,
        &TrickplayReady {
            item_id: id,
            interval_seconds: 10,
            tile_width: 320,
            tile_height: 180,
            grid_columns: 10,
            grid_rows: 10,
            thumbnail_count: 540,
            sheet_count: 6,
            source_size: c.file_size,
            source_modified_at: c.file_modified_at.as_ref(),
        },
    )
    .await
    .unwrap();
    assert!(
        trickplay::trickplay_candidates(pool, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let ready = trickplay::get_trickplay(pool, id).await.unwrap().unwrap();
    assert_eq!((ready.sheet_count, ready.thumbnail_count), (6, 540));

    // A replaced file (different size) is generated again.
    sqlx::query("UPDATE media_items SET file_size = 11 WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        trickplay::trickplay_candidates(pool, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    trickplay::save_trickplay_failed(pool, id, "broken", Some(11), None)
        .await
        .unwrap();
    assert!(
        trickplay::trickplay_candidates(pool, 10)
            .await
            .unwrap()
            .is_empty(),
        "failures wait for a file change"
    );
    assert!(trickplay::get_trickplay(pool, id).await.unwrap().is_none());
    let stats = trickplay::trickplay_stats(pool).await.unwrap();
    assert_eq!((stats.ready, stats.failed, stats.pending), (0, 1, 0));

    assert!(trickplay::reset_trickplay(pool, id).await.unwrap());
    assert_eq!(
        trickplay::trickplay_candidates(pool, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        !trickplay::reset_trickplay(pool, Uuid::new_v4())
            .await
            .unwrap()
    );
    db.drop().await;
}
