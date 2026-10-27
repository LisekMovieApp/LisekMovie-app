//! Database-backed tests for the watch statistics (Faza 13): totals,
//! rankings, filters and drop-off over known sessions, then the query plan
//! on a database with a lot of history.
//!
//! Opt-in: see `common/mod.rs`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, TimeDelta, Utc};
use lisek_db::PgPool;
use lisek_db::stats::{self, Content, StatsFilter};
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

async fn id_of(pool: &PgPool, sql: &str) -> Uuid {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(pool)
        .await
        .unwrap()
}

struct Session<'a> {
    user: Uuid,
    item: Uuid,
    started: DateTime<Utc>,
    watched: f64,
    position: f64,
    duration: f64,
    completed: bool,
    ended: bool,
    platform: &'a str,
    device: &'a str,
}

async fn insert(pool: &PgPool, s: Session<'_>) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO playback_sessions (user_id, item_id, auth_token_hash, ticket_hash, started_at, \
            last_heartbeat_at, ended_at, watched_seconds, position_seconds, duration_seconds, completed, \
            platform, device_type, paused) \
         VALUES ($1, $2, 'auth', gen_random_uuid()::text, $3, $3 + make_interval(secs => $4), \
            CASE WHEN $5 THEN $3 + make_interval(secs => $4) END, $4, $6, $7, $8, $9, $10, false) RETURNING id",
    )
    .bind(s.user)
    .bind(s.item)
    .bind(s.started)
    .bind(s.watched)
    .bind(s.ended)
    .bind(s.position)
    .bind(s.duration)
    .bind(s.completed)
    .bind(s.platform)
    .bind(s.device)
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn statistics_over_known_sessions() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let films = lisek_db::create_library(pool, "Filmy", "movies", &["/m".to_string()], false)
        .await
        .unwrap()
        .id;
    let tv = lisek_db::create_library(pool, "Seriale", "tv_shows", &["/t".to_string()], false)
        .await
        .unwrap()
        .id;
    let movie = |i: i32, title: &str| {
        ScanResult::Movie(ScannedMovie {
            file_path: PathBuf::from(format!("/m/{i}.mkv")),
            title: title.into(),
            year: Some(2000 + i),
            file: facts(),
        })
    };
    lisek_db::persist_scan_results(pool, films, &[movie(9, "Koralina"), movie(13, "Turbo")])
        .await
        .unwrap();
    let episodes: Vec<_> = (1..=2)
        .map(|n| {
            ScanResult::Episode(ScannedEpisode {
                file_path: PathBuf::from(format!("/t/got/S01E0{n}.mkv")),
                series_title: "Gra o tron".into(),
                season: 1,
                episode: n,
                end_episode: None,
                file: facts(),
            })
        })
        .collect();
    lisek_db::persist_scan_results(pool, tv, &episodes)
        .await
        .unwrap();

    let alice = id_of(
        pool,
        "INSERT INTO users (username, password_hash) VALUES ('alice', 'x') RETURNING id",
    )
    .await;
    let bob = id_of(
        pool,
        "INSERT INTO users (username, password_hash) VALUES ('bob', 'x') RETURNING id",
    )
    .await;
    let koralina = id_of(pool, "SELECT id FROM media_items WHERE title = 'Koralina'").await;
    let turbo = id_of(pool, "SELECT id FROM media_items WHERE title = 'Turbo'").await;
    let e1 = id_of(
        pool,
        "SELECT id FROM media_items WHERE file_path LIKE '%S01E01%'",
    )
    .await;
    let e2 = id_of(
        pool,
        "SELECT id FROM media_items WHERE file_path LIKE '%S01E02%'",
    )
    .await;
    let got = id_of(pool, "SELECT id FROM series WHERE title = 'Gra o tron'").await;

    let now = Utc::now();
    let day = |d: i64| now - TimeDelta::days(d);
    let base = |user, item, started, watched| Session {
        user,
        item,
        started,
        watched,
        position: watched,
        duration: 6000.0,
        completed: false,
        ended: true,
        platform: "windows",
        device: "desktop",
    };
    // Alice finishes Koralina on Windows; Bob gives up at 20 % on a TV.
    insert(
        pool,
        Session {
            completed: true,
            position: 5900.0,
            ..base(alice, koralina, day(1), 5880.0)
        },
    )
    .await;
    insert(
        pool,
        Session {
            position: 1250.0,
            platform: "smart_tv",
            device: "tv",
            ..base(bob, koralina, day(2), 1200.0)
        },
    )
    .await;
    // A 10-second peek: time, not a play.
    insert(pool, base(bob, turbo, day(2), 10.0)).await;
    // Two episodes on a phone, Bob still watching the second.
    insert(
        pool,
        Session {
            platform: "android",
            device: "mobile",
            completed: true,
            ..base(alice, e1, day(3), 3000.0)
        },
    )
    .await;
    let watching = insert(
        pool,
        Session {
            platform: "android",
            device: "mobile",
            ended: false,
            ..base(bob, e2, now - TimeDelta::seconds(90), 60.0)
        },
    )
    .await;
    sqlx::query("UPDATE playback_sessions SET last_heartbeat_at = now() WHERE id = $1")
        .bind(watching)
        .execute(pool)
        .await
        .unwrap();
    // Outside the week, and a session that never played: not counted.
    insert(pool, base(alice, turbo, day(20), 4000.0)).await;
    insert(pool, base(alice, turbo, day(1), 0.0)).await;

    let week = StatsFilter::range(now - TimeDelta::days(7), now + TimeDelta::seconds(1));
    let totals = stats::totals(pool, &week).await.unwrap();
    assert_eq!(
        totals.watched_seconds,
        5880.0 + 1200.0 + 10.0 + 3000.0 + 60.0
    );
    assert_eq!(
        (totals.plays, totals.users, totals.titles, totals.sessions),
        (4, 2, 4, 5)
    );
    assert_eq!(
        (totals.completed, totals.abandoned),
        (2, 2),
        "the peek and Bob's Koralina are abandoned"
    );

    let movies = stats::top_movies(pool, &week, 5).await.unwrap();
    assert_eq!(movies[0].title, "Koralina");
    assert_eq!(
        (movies[0].plays, movies[0].users, movies[0].completed),
        (2, 2, 1)
    );
    assert_eq!((movies[1].title.as_str(), movies[1].plays), ("Turbo", 0));
    let series = stats::top_series(pool, &week, 5).await.unwrap();
    assert_eq!(
        (
            series[0].title.as_str(),
            series[0].series_id,
            series[0].plays,
            series[0].users
        ),
        ("Gra o tron", Some(got), 2, 2)
    );
    assert_eq!(series[0].watched_seconds, 3060.0);

    let users = stats::top_users(pool, &week, 5).await.unwrap();
    assert_eq!(
        users
            .iter()
            .map(|u| u.username.as_str())
            .collect::<Vec<_>>(),
        ["alice", "bob"]
    );
    assert_eq!((users[0].watched_seconds, users[0].plays), (8880.0, 2));

    let content = stats::by_content(pool, &week).await.unwrap();
    assert_eq!(
        content
            .iter()
            .map(|c| (c.key.as_str(), c.plays))
            .collect::<Vec<_>>(),
        [("movie", 2), ("series", 2)]
    );
    let platforms = stats::by_platform(pool, &week).await.unwrap();
    assert_eq!(platforms[0].key, "windows");
    assert!(
        platforms
            .iter()
            .any(|p| p.key == "smart_tv" && p.watched_seconds == 1200.0)
    );

    // Bob stopped Koralina at 1250 / 6000 = 20.8 %; the peek at 10 / 6000.
    let dropoff = stats::dropoff(pool, &week).await.unwrap();
    assert_eq!(dropoff, vec![1, 0, 1, 0, 0, 0, 0, 0, 0, 0]);

    let list = stats::sessions(pool, &week, 50, 0).await.unwrap();
    assert_eq!(list.len(), 5);
    assert_eq!(
        (list[0].outcome.as_str(), list[0].season, list[0].episode),
        ("watching", Some(1), Some(2))
    );
    assert!(list.iter().any(|s| s.username == "bob"
        && s.title == "Koralina"
        && s.outcome == "abandoned"
        && s.position_seconds == 1250.0));

    // Filters.
    let f = StatsFilter {
        user_id: Some(bob),
        ..week.clone()
    };
    assert_eq!(stats::totals(pool, &f).await.unwrap().sessions, 3);
    let f = StatsFilter {
        platform: Some("smart_tv".into()),
        ..week.clone()
    };
    assert_eq!(
        stats::totals(pool, &f).await.unwrap().watched_seconds,
        1200.0
    );
    let f = StatsFilter {
        device_type: Some("mobile".into()),
        ..week.clone()
    };
    assert_eq!(stats::totals(pool, &f).await.unwrap().sessions, 2);
    let f = StatsFilter {
        content: Some(Content::Series),
        ..week.clone()
    };
    assert!(stats::top_movies(pool, &f, 5).await.unwrap().is_empty());
    let f = StatsFilter {
        item: Some(got),
        ..week.clone()
    };
    assert_eq!(
        stats::totals(pool, &f).await.unwrap().sessions,
        2,
        "a series id covers its episodes"
    );
    let f = StatsFilter {
        item: Some(koralina),
        ..week.clone()
    };
    assert_eq!(stats::dropoff(pool, &f).await.unwrap()[2], 1);

    let days = stats::by_day(pool, &week, "Europe/Warsaw").await.unwrap();
    assert_eq!(
        days.iter().map(|d| d.watched_seconds).sum::<f64>(),
        totals.watched_seconds
    );
    assert!(stats::valid_timezone(pool, "Europe/Warsaw").await);
    assert!(!stats::valid_timezone(pool, "Mars/Olympus").await);
    let hours = stats::by_hour(pool, &week, "UTC").await.unwrap();
    assert_eq!(hours.iter().map(|h| h.sessions).sum::<i64>(), 5);

    // The bots' key.
    assert!(stats::api_key(pool).await.unwrap().is_none());
    stats::set_api_key(pool, "hash1").await.unwrap();
    stats::set_api_key(pool, "hash2").await.unwrap();
    assert_eq!(stats::api_key(pool).await.unwrap().unwrap().0, "hash2");
    stats::delete_api_key(pool).await.unwrap();
    assert!(stats::api_key(pool).await.unwrap().is_none());

    // Scale: two years of sessions. A week's statistics read the week
    // through the time index, not the whole table.
    sqlx::query(
        "INSERT INTO playback_sessions (user_id, item_id, auth_token_hash, ticket_hash, started_at, watched_seconds, \
            position_seconds, duration_seconds, ended_at, platform, device_type) \
         SELECT CASE WHEN g % 2 = 0 THEN $1 ELSE $2 END, (ARRAY[$3, $4, $5, $6]::uuid[])[1 + g % 4], 'auth', \
                'bulk' || g, now() - interval '400 days' - (g || ' minutes')::interval, 600, 600, 6000, now(), 'linux', 'desktop' \
         FROM generate_series(1, 60000) g",
    )
    .bind(alice)
    .bind(bob)
    .bind(koralina)
    .bind(turbo)
    .bind(e1)
    .bind(e2)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE playback_sessions")
        .execute(pool)
        .await
        .unwrap();
    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT count(*) FROM playback_sessions p WHERE p.started_at >= $1 AND p.started_at < $2 AND p.watched_seconds > 0",
    )
    .bind(week.from)
    .bind(week.to)
    .fetch_all(pool)
    .await
    .unwrap();
    assert!(
        plan.iter().any(|l| l.contains("playback_sessions_started")),
        "{plan:#?}"
    );
    let started = std::time::Instant::now();
    let again = stats::totals(pool, &week).await.unwrap();
    assert_eq!(again.sessions, 5, "the old bulk is outside the week");
    stats::top_movies(pool, &week, 5).await.unwrap();
    stats::sessions(pool, &week, 100, 0).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    db.drop().await;
}
