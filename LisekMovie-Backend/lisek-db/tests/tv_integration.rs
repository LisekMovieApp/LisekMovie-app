//! Database-backed tests for Faza 26 TV channels: the timeline survives a
//! restart, rebuilds are atomic and serialized, a refresh keeps what plays
//! now, and the schedule's sources resolve to real items.
//! Opt-in: see `common/mod.rs`.

use chrono::{DateTime, Duration, Utc};
use lisek_db::tv::{self, ChannelInput, FillerPresetRow, RebuildMode, SlotInput};
use sqlx::PgPool;
use uuid::Uuid;

mod common;
use common::TestDb;

async fn library(pool: &PgPool, name: &str, kind: &str) -> Uuid {
    lisek_db::create_library(pool, name, kind, &["/media/x".to_string()], false).await.unwrap().id
}

async fn movie(pool: &PgPool, lib: Uuid, title: &str, minutes: f64) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO media_items (library_id, item_type, title, file_path, metadata, duration_seconds) \
         VALUES ($1, 'movie', $2, $3, '{\"year\": 2001}', $4) RETURNING id",
    )
    .bind(lib)
    .bind(title)
    .bind(format!("/media/x/{title}.mkv"))
    .bind(minutes * 60.0)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn episode(pool: &PgPool, lib: Uuid, show: &str, number: i32, minutes: f64) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO media_items (library_id, item_type, title, file_path, metadata, duration_seconds) \
         VALUES ($1, 'episode', $2, $3, jsonb_build_object('season', 1, 'episode', $4::int), $5) RETURNING id",
    )
    .bind(lib)
    .bind(show)
    .bind(format!("/media/x/{show}.S01E{number:02}.mkv"))
    .bind(number)
    .bind(minutes * 60.0)
    .fetch_one(pool)
    .await
    .unwrap()
}

fn channel_input(number: &str, name: &str) -> ChannelInput {
    ChannelInput {
        number: number.into(),
        name: name.into(),
        group_title: None,
        enabled: true,
        timezone: "Europe/Warsaw".into(),
        quality: "720p".into(),
        fallback_filler_id: None,
        keep_running: false,
    }
}

fn slot(kind: &str, id: Uuid, mode: &str) -> SlotInput {
    SlotInput {
        start_time: None,
        start_mode: "strict".into(),
        days_of_week: 127,
        source_kind: kind.into(),
        source_id: id,
        play_order: "season_episode".into(),
        mode: mode.into(),
        count: (mode == "multiple").then_some(2),
        duration_minutes: (mode == "duration").then_some(60),
        tail_mode: "offline".into(),
        pre_filler_id: None,
        mid_filler_id: None,
        post_filler_id: None,
        tail_filler_id: None,
        title_override: None,
        in_guide: true,
    }
}

async fn all_items(pool: &PgPool, channel: Uuid) -> Vec<(DateTime<Utc>, DateTime<Utc>, String)> {
    sqlx::query_as("SELECT starts_at, ends_at, kind FROM tv_playout_items WHERE channel_id = $1 ORDER BY starts_at")
        .bind(channel)
        .fetch_all(pool)
        .await
        .unwrap()
}

fn assert_no_overlap(items: &[(DateTime<Utc>, DateTime<Utc>, String)]) {
    for pair in items.windows(2) {
        assert!(pair[0].1 <= pair[1].0, "overlap: {:?} / {:?}", pair[0], pair[1]);
    }
}

#[tokio::test]
async fn a_channel_plays_its_schedule_and_survives_a_restart() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let shows = library(pool, "Seriale", "tv_shows").await;
    for n in 1..=3 {
        episode(pool, shows, "Zdrowie", n, 40.0).await;
    }
    let series: Uuid = sqlx::query_scalar("SELECT id FROM series WHERE title = 'Zdrowie'").fetch_one(pool).await.unwrap();

    let channel = tv::create_channel(pool, &channel_input("7", "Siódemka")).await.unwrap();
    assert!(tv::number_taken(pool, "7", None).await.unwrap());
    assert!(!tv::number_taken(pool, "7", Some(channel.id)).await.unwrap());
    assert_eq!(tv::next_free_number(pool).await.unwrap(), "1");
    let mut flood = slot("series", series, "flood");
    flood.start_time = Some("06:00".into());
    assert!(tv::replace_slots(pool, channel.id, &[flood]).await.unwrap());

    let now: DateTime<Utc> = "2026-11-16T10:10:00Z".parse().unwrap();
    let due = tv::builds_due(pool, now).await.unwrap();
    assert_eq!(due, vec![(channel.id, RebuildMode::Reset)], "a new channel is planned from scratch");
    let result = tv::rebuild(pool, channel.id, RebuildMode::Reset, now).await.unwrap().unwrap();
    assert!(result.planned > 100, "72 hours of 40 minute episodes");
    assert!(tv::builds_due(pool, now).await.unwrap().is_empty());

    let entry = tv::item_at(pool, channel.id, now + Duration::minutes(5)).await.unwrap().unwrap();
    assert_eq!(entry.title, "Zdrowie");
    assert_eq!(entry.subtitle.as_deref(), Some("S01E01"));
    assert_eq!(entry.path.as_deref(), Some("/media/x/Zdrowie.S01E01.mkv"));
    assert_eq!(entry.starts_at, now, "a flood in progress starts at the first plan");
    let next = tv::item_after(pool, channel.id, now + Duration::minutes(5)).await.unwrap().unwrap();
    assert_eq!(next.subtitle.as_deref(), Some("S01E02"));
    assert_eq!(next.starts_at, entry.ends_at);
    assert_no_overlap(&all_items(pool, channel.id).await);

    // A new connection pool: a restarted server. The same answer, no rebuild.
    let again = db.reconnect().await;
    let same = tv::item_at(&again, channel.id, now + Duration::minutes(5)).await.unwrap().unwrap();
    assert_eq!((same.starts_at, same.title.as_str(), same.subtitle.as_deref()), (entry.starts_at, "Zdrowie", Some("S01E01")));
    assert!(tv::builds_due(&again, now).await.unwrap().is_empty());
    again.close().await;
    db.drop().await;
}

#[tokio::test]
async fn continue_extends_without_touching_what_is_planned() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = library(pool, "Filmy", "movies").await;
    for (i, title) in ["A", "B", "C", "D"].iter().enumerate() {
        movie(pool, lib, title, 90.0 + i as f64).await;
    }
    let channel = tv::create_channel(pool, &channel_input("1", "Kino")).await.unwrap();
    let mut s = slot("library", lib, "flood");
    s.play_order = "shuffle".into();
    tv::replace_slots(pool, channel.id, &[s]).await.unwrap();

    let t0: DateTime<Utc> = "2026-11-16T10:00:00Z".parse().unwrap();
    tv::rebuild(pool, channel.id, RebuildMode::Reset, t0).await.unwrap();
    let before = all_items(pool, channel.id).await;

    // Two days later the horizon moved: only the end is added.
    let t1 = t0 + Duration::hours(30);
    assert_eq!(tv::builds_due(pool, t1).await.unwrap(), vec![(channel.id, RebuildMode::Continue)]);
    tv::rebuild(pool, channel.id, RebuildMode::Continue, t1).await.unwrap();
    let after = all_items(pool, channel.id).await;
    // (Entries that ended more than a day ago are pruned.)
    let kept: Vec<_> = before.iter().filter(|i| i.1 > t1 - Duration::hours(24)).cloned().collect();
    assert_eq!(&after[..kept.len()], &kept[..], "nothing already planned changed");
    assert!(after.len() > before.len());
    assert!(after.last().unwrap().1 >= t1 + Duration::hours(71));
    assert_no_overlap(&after);
    db.drop().await;
}

#[tokio::test]
async fn a_refresh_keeps_the_current_item_and_replans_the_rest() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = library(pool, "Filmy", "movies").await;
    let a = movie(pool, lib, "Alfa", 100.0).await;
    let b = movie(pool, lib, "Beta", 100.0).await;
    let channel = tv::create_channel(pool, &channel_input("2", "Dwójka")).await.unwrap();
    tv::replace_slots(pool, channel.id, &[slot("movie", a, "flood")]).await.unwrap();
    let t0: DateTime<Utc> = "2026-11-16T10:00:00Z".parse().unwrap();
    tv::rebuild(pool, channel.id, RebuildMode::Reset, t0).await.unwrap();

    let now = t0 + Duration::minutes(130); // inside the second showing of Alfa
    let playing = tv::item_at(pool, channel.id, now).await.unwrap().unwrap();
    assert_eq!(playing.title, "Alfa");

    // The administrator swaps the source for another film.
    tv::replace_slots(pool, channel.id, &[slot("movie", b, "flood")]).await.unwrap();
    assert_eq!(tv::builds_due(pool, now).await.unwrap(), vec![(channel.id, RebuildMode::Refresh)]);
    tv::rebuild(pool, channel.id, RebuildMode::Refresh, now).await.unwrap();

    let still = tv::item_at(pool, channel.id, now).await.unwrap().unwrap();
    assert_eq!((still.title.as_str(), still.starts_at), ("Alfa", playing.starts_at), "the viewer's film is not cut off");
    let next = tv::item_after(pool, channel.id, now).await.unwrap().unwrap();
    assert_eq!((next.title.as_str(), next.starts_at), ("Beta", playing.ends_at), "and the new schedule follows it");
    assert_no_overlap(&all_items(pool, channel.id).await);
    assert!(tv::builds_due(pool, now).await.unwrap().is_empty());
    db.drop().await;
}

#[tokio::test]
async fn concurrent_rebuilds_queue_up_and_leave_one_clean_timeline() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = library(pool, "Filmy", "movies").await;
    for n in 0..5 {
        movie(pool, lib, &format!("Film {n}"), 95.0).await;
    }
    let channel = tv::create_channel(pool, &channel_input("3", "Trójka")).await.unwrap();
    tv::replace_slots(pool, channel.id, &[slot("library", lib, "flood")]).await.unwrap();
    let now: DateTime<Utc> = "2026-11-16T10:00:00Z".parse().unwrap();

    let mut tasks = Vec::new();
    for i in 0..6 {
        let pool = pool.clone();
        let mode = if i % 2 == 0 { RebuildMode::Refresh } else { RebuildMode::Continue };
        tasks.push(tokio::spawn(async move { tv::rebuild(&pool, channel.id, mode, now).await }));
    }
    // A reader polls while they run: every answer is a whole timeline.
    let reader = {
        let pool = pool.clone();
        tokio::spawn(async move {
            for _ in 0..40 {
                let items: Vec<(DateTime<Utc>, DateTime<Utc>)> =
                    sqlx::query_as("SELECT starts_at, ends_at FROM tv_playout_items WHERE channel_id = $1 ORDER BY starts_at").bind(channel.id).fetch_all(&pool).await.unwrap();
                for pair in items.windows(2) {
                    assert!(pair[0].1 <= pair[1].0, "a reader saw overlapping entries");
                }
                if let Some(last) = items.last() {
                    assert!(last.1 - items[0].0 >= Duration::hours(70), "a reader saw a partly built timeline");
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
    };
    for task in tasks {
        task.await.unwrap().unwrap().unwrap();
    }
    reader.await.unwrap();
    let items = all_items(pool, channel.id).await;
    assert_no_overlap(&items);
    assert!(items.last().unwrap().1 - items[0].0 >= Duration::hours(70));
    db.drop().await;
}

#[tokio::test]
async fn fillers_come_from_folders_and_are_planned_around_films() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = library(pool, "Filmy", "movies").await;
    let film = movie(pool, lib, "Film", 22.0).await;
    let folder = tv::add_folder(pool, "/media/reklamy").await.unwrap().unwrap();
    assert!(tv::add_folder(pool, "/media/reklamy").await.unwrap().is_none(), "no duplicates");
    tv::replace_folder_files(pool, folder.id, &[("/media/reklamy/a.mp4".into(), 30_000), ("/media/reklamy/b.mp4".into(), 45_000)], None).await.unwrap();
    let files = tv::folder_files(pool, folder.id).await.unwrap();
    assert_eq!(files.len(), 2);
    // A rescan keeps ids of files that are still there.
    tv::replace_folder_files(pool, folder.id, &[("/media/reklamy/a.mp4".into(), 30_000)], None).await.unwrap();
    let after = tv::folder_files(pool, folder.id).await.unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, files.iter().find(|f| f.path.ends_with("a.mp4")).unwrap().id);

    let preset = tv::save_preset(
        pool,
        &FillerPresetRow { id: Uuid::nil(), name: "Do pół godziny".into(), kind: "post".into(), mode: "pad".into(), count: None, duration_seconds: None, pad_minutes: Some(30), folder_ids: vec![folder.id] },
    )
    .await
    .unwrap()
    .unwrap();
    let clash = FillerPresetRow { id: Uuid::nil(), name: "do pół GODZINY".into(), ..preset.clone() };
    assert!(tv::save_preset(pool, &clash).await.unwrap().is_none(), "names are unique, ignoring case");

    let channel = tv::create_channel(pool, &channel_input("4", "Czwórka")).await.unwrap();
    let mut s = slot("movie", film, "multiple");
    s.count = Some(2);
    s.post_filler_id = Some(preset.id);
    tv::replace_slots(pool, channel.id, &[s]).await.unwrap();
    let now: DateTime<Utc> = "2026-11-16T09:00:00Z".parse().unwrap(); // 10:00 in Warsaw
    tv::rebuild(pool, channel.id, RebuildMode::Reset, now).await.unwrap();
    let entries = tv::items_between(pool, channel.id, now, now + Duration::hours(2)).await.unwrap();
    let kinds: Vec<_> = entries.iter().map(|e| e.kind.as_db()).collect();
    assert_eq!(kinds[0], "content");
    let second = kinds[1..].iter().position(|k| *k == "content").unwrap() + 1;
    assert!(kinds[1..second].iter().all(|k| *k == "post"), "{kinds:?}");
    assert_eq!(entries[second - 1].ends_at, now + Duration::minutes(30), "padded to the half hour of the local clock");
    assert_eq!(entries[second].starts_at, now + Duration::minutes(30));
    assert_eq!(entries[1].path.as_deref(), Some("/media/reklamy/a.mp4"));

    let programmes = tv::guide(pool, channel.id, now, now + Duration::hours(2)).await.unwrap();
    assert_eq!((programmes[0].title.as_str(), programmes[0].starts_at, programmes[0].ends_at), ("Film", now, now + Duration::minutes(30)));

    // Deleting the folder removes the files and the preset's link to it.
    assert!(tv::delete_folder(pool, folder.id).await.unwrap());
    assert!(tv::list_presets(pool).await.unwrap()[0].folder_ids.is_empty());
    assert_eq!(tv::builds_due(pool, now).await.unwrap(), vec![(channel.id, RebuildMode::Refresh)], "the channel is replanned without the ads");
    db.drop().await;
}

#[tokio::test]
async fn schedules_are_copied_and_sources_searched() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = library(pool, "Filmy", "movies").await;
    let film = movie(pool, lib, "Pan Tadeusz", 100.0).await;
    episode(pool, library(pool, "Seriale", "tv_shows").await, "Pan Samochodzik", 1, 40.0).await;

    let hits = tv::search_sources(pool, "pan ", 10).await.unwrap();
    let kinds: Vec<(&str, &str)> = hits.iter().map(|h| (h.kind.as_str(), h.title.as_str())).collect();
    assert!(kinds.contains(&("movie", "Pan Tadeusz")) && kinds.contains(&("series", "Pan Samochodzik")), "{kinds:?}");
    assert!(tv::search_sources(pool, "100%", 10).await.unwrap().is_empty(), "LIKE wildcards are literal");

    let a = tv::create_channel(pool, &channel_input("1", "A")).await.unwrap();
    let b = tv::create_channel(pool, &channel_input("2", "B")).await.unwrap();
    let mut s = slot("movie", film, "one");
    s.start_time = Some("20:30".into());
    tv::replace_slots(pool, a.id, &[s, slot("library", lib, "flood")]).await.unwrap();
    let before = tv::get_channel(pool, b.id).await.unwrap().unwrap().schedule_version;
    assert!(tv::copy_schedule(pool, a.id, b.id).await.unwrap());
    let copied = tv::list_slots(pool, b.id).await.unwrap();
    assert_eq!(copied.len(), 2);
    assert_eq!(copied[0].slot.start_time.as_deref(), Some("20:30"));
    assert_eq!(copied[0].source_label.as_deref(), Some("Pan Tadeusz"));
    assert_eq!(copied[1].source_label.as_deref(), Some("Filmy"));
    assert_ne!(copied[0].id, tv::list_slots(pool, a.id).await.unwrap()[0].id, "new slots, not shared ones");
    assert!(tv::get_channel(pool, b.id).await.unwrap().unwrap().schedule_version > before);
    assert!(!tv::copy_schedule(pool, a.id, Uuid::new_v4()).await.unwrap());

    // A source that was deleted plays nothing, and the schedule still builds.
    sqlx::query("DELETE FROM media_items WHERE id = $1").bind(film).execute(pool).await.unwrap();
    let result = tv::rebuild(pool, a.id, RebuildMode::Reset, Utc::now()).await.unwrap().unwrap();
    assert!(result.warnings.iter().any(|w| w.contains("slot 1")), "{:?}", result.warnings);
    // Whole-database invariants: deleting a channel takes its rows along.
    assert!(tv::delete_channel(pool, a.id).await.unwrap().is_some());
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM tv_slots WHERE channel_id = $1").bind(a.id).fetch_one(pool).await.unwrap();
    assert_eq!(left, 0);
    db.drop().await;
}

#[tokio::test]
async fn a_removed_film_leaves_a_gap_not_a_broken_timeline() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = library(pool, "Filmy", "movies").await;
    let a = movie(pool, lib, "Alfa", 50.0).await;
    let channel = tv::create_channel(pool, &channel_input("5", "Piątka")).await.unwrap();
    tv::replace_slots(pool, channel.id, &[slot("movie", a, "flood")]).await.unwrap();
    let now: DateTime<Utc> = "2026-11-16T10:00:00Z".parse().unwrap();
    tv::rebuild(pool, channel.id, RebuildMode::Reset, now).await.unwrap();
    sqlx::query("DELETE FROM media_items WHERE id = $1").bind(a).execute(pool).await.unwrap();
    let entry = tv::item_at(pool, channel.id, now + Duration::minutes(10)).await.unwrap().unwrap();
    assert_eq!(entry.title, "Alfa");
    assert!(entry.path.is_none(), "the entry stays for the guide; the stream sees no file and shows the fallback");
    db.drop().await;
}
