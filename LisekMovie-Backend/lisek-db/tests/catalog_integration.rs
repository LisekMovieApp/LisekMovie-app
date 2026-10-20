//! Database-backed tests for the series triggers, scan persistence, the
//! catalog queries and the scan queue.
//!
//! Opt-in: see `common/mod.rs`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lisek_core::media_probe::{MediaProbe, MediaStream, StreamKind};
use lisek_db::streams::FileStamp;
use lisek_db::{LibraryFilter, LibrarySort, PgPool};
use lisek_library::{FileFacts, ScanResult, ScannedEpisode, ScannedMovie};
use uuid::Uuid;

mod common;
use common::TestDb;

fn facts(size: u64, secs: u64) -> FileFacts {
    FileFacts {
        size_bytes: Some(size),
        modified: Some(SystemTime::UNIX_EPOCH + Duration::new(secs, 987_654_321)),
    }
}

fn movie(path: &str, title: &str, year: i32, f: FileFacts) -> ScanResult {
    ScanResult::Movie(ScannedMovie { file_path: PathBuf::from(path), title: title.into(), year: Some(year), file: f })
}

fn episode(path: &str, show: &str, season: i32, number: i32, f: FileFacts) -> ScanResult {
    ScanResult::Episode(ScannedEpisode {
        file_path: PathBuf::from(path),
        series_title: show.into(),
        season,
        episode: number,
        end_episode: None,
        file: f,
    })
}

async fn library(pool: &PgPool, kind: &str) -> Uuid {
    lisek_db::create_library(pool, kind, kind, &["/media".to_string()], false).await.unwrap().id
}

#[tokio::test]
async fn scan_persistence_builds_series_and_tracks_file_changes() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let tv = library(pool, "tv_shows").await;

    let first = vec![
        episode("/media/Show/S01E01.mkv", "Show", 1, 1, facts(10, 1)),
        episode("/media/Show/S01E02.mkv", "Show", 1, 2, facts(11, 1)),
        episode("/media/Show/S02E01.mkv", "Show", 2, 1, facts(12, 1)),
        // Overlapping root paths report a file twice; it must be written once.
        episode("/media/Show/S02E01.mkv", "Show", 2, 1, facts(12, 1)),
        episode("/media/Other/S01E01.mkv", "Other", 1, 1, facts(13, 1)),
        ScanResult::Unrecognized(PathBuf::from("/media/junk.mkv")),
    ];
    let mut progress = Vec::new();
    let persisted =
        lisek_db::persist_scan_results_with_progress(pool, tv, &first, |n| progress.push(n)).await.unwrap();
    assert_eq!(persisted.summary.episodes_found, 4);
    assert_eq!(persisted.summary.unrecognized, 1);
    assert_eq!(progress, vec![4]);
    assert_eq!(persisted.enrich_targets.len(), 4);
    assert_eq!(persisted.probe_targets.len(), 4, "new files always need a probe");

    let (series, seasons, episodes): (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM series), (SELECT count(*) FROM seasons), (SELECT count(*) FROM episodes)",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!((series, seasons, episodes), (2, 3, 4));

    // Catalog: one card per show, the card resolves back to the show.
    let page = lisek_db::list_items_by_library_page(pool, tv, LibrarySort::TitleAsc, LibraryFilter::default(), 10, 0)
        .await
        .unwrap();
    assert_eq!(page.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Other", "Show"]);
    assert!(page.iter().all(|i| i.item_type == "series"));
    let show_card = page[1].id;
    let seasons = lisek_db::list_series_seasons(pool, show_card).await.unwrap();
    assert_eq!(seasons.iter().map(|s| (s.season, s.episodes.len())).collect::<Vec<_>>(), [(1, 2), (2, 1)]);
    let detail = lisek_db::get_media_item(pool, seasons[0].episodes[0].id).await.unwrap().unwrap();
    assert_eq!((detail.id, detail.item_type.as_str()), (show_card, "series"));
    let badges = lisek_db::classify_series_badges(pool, &[show_card]).await.unwrap();
    assert_eq!(badges.get(&show_card), Some(&lisek_db::SeriesBadge::NewSeries));

    // Probe one file, then rescan: unchanged files are not re-probed, a
    // changed one is.
    let probed_id = persisted.probe_targets[0].0;
    let probe = MediaProbe {
        container: Some("matroska,webm".into()),
        duration_seconds: Some(1200.5),
        bitrate_kbps: Some(4000),
        start_time_seconds: Some(1.5),
        streams: vec![MediaStream {
            index: 0,
            kind: StreamKind::Video,
            codec: Some("h264".into()),
            profile: None,
            language: None,
            title: None,
            is_default: true,
            is_forced: false,
            width: Some(1920),
            height: Some(1080),
            bit_depth: Some(8),
            pixel_format: Some("yuv420p".into()),
            color_transfer: None,
            color_primaries: None,
            color_space: None,
            field_order: Some("progressive".into()),
            frame_rate: Some(23.976),
            channels: None,
            channel_layout: None,
            sample_rate: None,
            bitrate_kbps: None,
            dovi_profile: Some(8),
        }],
    };
    lisek_db::streams::save_probe(pool, probed_id, &probe, None).await.unwrap();
    let (stored, stamp) = lisek_db::streams::stored_probe(pool, probed_id).await.unwrap().unwrap();
    assert_eq!(stored, probe);
    assert!(stamp.size.is_some() && stamp.modified.is_some());

    // Keyframe cache: valid for the same file, ignored once it changed.
    let keyframes = [0.0, 2.002, 4.004];
    lisek_db::streams::save_keyframes(pool, probed_id, Some(stamp), &keyframes).await.unwrap();
    let cached = lisek_db::streams::stored_keyframes(pool, probed_id, Some(stamp)).await.unwrap();
    assert_eq!(cached.as_deref(), Some(&keyframes[..]));
    let changed_stamp = lisek_db::streams::FileStamp { size: stamp.size.map(|s| s + 1), ..stamp };
    assert_eq!(lisek_db::streams::stored_keyframes(pool, probed_id, Some(changed_stamp)).await.unwrap(), None);

    let rescan = lisek_db::persist_scan_results(pool, tv, &first).await.unwrap();
    assert!(!rescan.probe_targets.iter().any(|(id, _)| *id == probed_id));
    let path = persisted.probe_targets[0].1.to_string_lossy().into_owned();
    let changed: Vec<ScanResult> = first
        .iter()
        .map(|r| match r {
            ScanResult::Episode(e) if e.file_path.to_string_lossy() == path => {
                let mut e = e.clone();
                e.file = facts(999, 2);
                ScanResult::Episode(e)
            }
            other => other.clone(),
        })
        .collect();
    let rescan = lisek_db::persist_scan_results(pool, tv, &changed).await.unwrap();
    assert!(rescan.probe_targets.iter().any(|(id, _)| *id == probed_id));
    assert!(lisek_db::streams::stored_probe(pool, probed_id).await.unwrap().is_none());

    // FileStamp built from fs metadata compares equal to what the scan stored.
    let file = tempfile_with_bytes(b"abc");
    let stamp = FileStamp::from_metadata(&std::fs::metadata(&file).unwrap());
    assert!(!stamp.differs_from(&stamp));

    // Renaming a show in the editor moves its episode to another series;
    // deleting the library removes every series row.
    sqlx::query("UPDATE media_items SET title = 'Renamed' WHERE title = 'Other'").execute(pool).await.unwrap();
    let titles: Vec<String> = sqlx::query_scalar("SELECT title FROM series ORDER BY title").fetch_all(pool).await.unwrap();
    assert_eq!(titles, ["Renamed", "Show"]);
    lisek_db::delete_library(pool, tv).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM series").fetch_one(pool).await.unwrap();
    assert_eq!(left, 0);

    db.drop().await;
}

fn tempfile_with_bytes(bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("lisek-stamp-{}", Uuid::new_v4()));
    std::fs::write(&path, bytes).unwrap();
    path
}

#[tokio::test]
async fn locked_items_keep_metadata_but_refresh_file_facts() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let movies = library(pool, "movies").await;
    let scan = vec![movie("/media/a.mkv", "Alpha", 2001, facts(1, 1))];
    let persisted = lisek_db::persist_scan_results(pool, movies, &scan).await.unwrap();
    let id = persisted.enrich_targets[0].0;
    lisek_db::update_media_item(pool, id, "Alpha (Director's Cut)", None, Some(2002), None, true, None, &[])
        .await
        .unwrap();

    let renamed = vec![movie("/media/a.mkv", "Alpha Renamed", 1999, facts(2, 2))];
    let again = lisek_db::persist_scan_results(pool, movies, &renamed).await.unwrap();
    assert!(again.enrich_targets.is_empty(), "locked items are never re-enriched");
    assert_eq!(again.probe_targets.len(), 1, "but their changed file is re-probed");
    let (title, year, size): (String, Option<i32>, Option<i64>) =
        sqlx::query_as("SELECT title, year, file_size FROM media_items WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!((title.as_str(), year, size), ("Alpha (Director's Cut)", Some(2002), Some(2)));
    db.drop().await;
}

#[tokio::test]
async fn catalog_queries_merge_movies_and_series_correctly() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let movies = library(pool, "movies").await;
    let tv = library(pool, "tv_shows").await;
    let mut scan = Vec::new();
    for (i, title) in ["Matrix", "Memento", "alien", "Zodiac", "50% Off_Sale"].iter().enumerate() {
        scan.push(movie(&format!("/media/m{i}.mkv"), title, 1990 + i as i32, facts(1, 1)));
    }
    lisek_db::persist_scan_results(pool, movies, &scan).await.unwrap();
    lisek_db::persist_scan_results(pool, tv, &[episode("/media/s/e1.mkv", "Mandalorian", 1, 1, facts(1, 1))])
        .await
        .unwrap();

    let letter = |c| LibraryFilter { letter: Some(c), search: None };
    let m = lisek_db::list_items_by_library_page(pool, movies, LibrarySort::TitleAsc, letter('m'), 10, 0).await.unwrap();
    assert_eq!(m.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Matrix", "Memento"]);
    assert_eq!(lisek_db::count_items_by_library(pool, movies, letter('A')).await.unwrap(), 1, "case-insensitive");
    let search = LibraryFilter { letter: None, search: Some("% off_") };
    let found = lisek_db::list_items_by_library_page(pool, movies, LibrarySort::TitleAsc, search, 10, 0).await.unwrap();
    assert_eq!(found.len(), 1, "wildcards in the search term are literal");
    let desc = lisek_db::list_items_by_library_page(pool, movies, LibrarySort::TitleDesc, LibraryFilter::default(), 2, 1)
        .await
        .unwrap();
    assert_eq!(desc.len(), 2);

    let recent = lisek_db::list_recently_added(pool, None, 3).await.unwrap();
    assert_eq!(recent[0].title, "Mandalorian", "the newest scan comes first");
    let top = lisek_db::list_top_today(pool, 10).await.unwrap();
    assert_eq!(top.first().map(|i| i.year), Some(Some(1994)));
    assert_eq!(top.last().map(|i| i.item_type.as_str()), Some("series"), "undated shows sort last");
    for _ in 0..20 {
        let sample = lisek_db::list_recommended(pool, None, 4).await.unwrap();
        assert_eq!(sample.len(), 4);
        let mut ids: Vec<_> = sample.iter().map(|i| i.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 4, "no duplicates");
    }
    let everything = lisek_db::list_popular(pool, Some(movies), 50).await.unwrap();
    assert_eq!(everything.len(), 5, "a limit above the library size returns each item once");
    assert!(lisek_db::get_hero_item(pool).await.unwrap().is_some());

    let (job, created) = lisek_db::scan_jobs::enqueue(pool, movies, None).await.unwrap();
    assert!(created);
    let (again, created_again) = lisek_db::scan_jobs::enqueue(pool, movies, None).await.unwrap();
    assert_eq!((again.id, created_again), (job.id, false), "one pending job per library");
    let claimed = lisek_db::scan_jobs::claim_next(pool).await.unwrap().unwrap();
    assert_eq!((claimed.id, claimed.status.as_str()), (job.id, "running"));
    assert_eq!(lisek_db::scan_jobs::requeue_interrupted(pool).await.unwrap(), 1);
    db.drop().await;
}
