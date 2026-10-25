//! Database-backed tests for playback progress reports (resume position,
//! viewing time, quality counters). Opt-in: see `common/mod.rs`.

use std::path::PathBuf;

use lisek_db::playback::{self, ProgressUpdate};
use lisek_library::{FileFacts, ScanResult, ScannedMovie};
use uuid::Uuid;

mod common;
use common::TestDb;

fn update(position: f64, playing: bool) -> ProgressUpdate {
    ProgressUpdate {
        position,
        duration: 6000.0,
        playing,
        max_counted_gap: 35.0,
        ..Default::default()
    }
}

#[tokio::test]
async fn progress_reports_store_resume_viewing_time_and_quality() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let library = lisek_db::create_library(pool, "Filmy", "movies", &["/media".to_string()], false)
        .await
        .unwrap()
        .id;
    let film = ScanResult::Movie(ScannedMovie {
        file_path: PathBuf::from("/media/Film (2020).mkv"),
        title: "Film".into(),
        year: Some(2020),
        file: FileFacts {
            size_bytes: Some(1),
            modified: None,
        },
    });
    lisek_db::persist_scan_results(pool, library, &[film])
        .await
        .unwrap();
    let (item,): (Uuid,) = sqlx::query_as("SELECT id FROM media_items")
        .fetch_one(pool)
        .await
        .unwrap();
    let (user,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (username, password_hash) VALUES ('viewer', 'x') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let (session,): (Uuid,) = sqlx::query_as(
        "INSERT INTO playback_sessions (user_id, item_id, auth_token_hash, ticket_hash) VALUES ($1, $2, 'a', 't') RETURNING id",
    )
    .bind(user)
    .bind(item)
    .fetch_one(pool)
    .await
    .unwrap();

    // Playing at 45:00, 20 s after the previous report: counted, resumable.
    sqlx::query("UPDATE playback_sessions SET paused = false, last_heartbeat_at = now() - interval '20 seconds' WHERE id = $1")
        .bind(session)
        .execute(pool)
        .await
        .unwrap();
    let mut report = update(2700.0, true);
    report.stalls = 2;
    report.stall_seconds = 1.5;
    report.bandwidth_kbps = Some(24_000);
    let outcome = playback::record_progress(pool, user, session, &report)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.resume_position, 2700.0);
    assert!(!outcome.watched);
    let state = playback::media_state(pool, user, item).await.unwrap();
    assert_eq!(
        (state.position_seconds, state.duration_seconds),
        (2700.0, 6000.0)
    );

    // A ten-minute gap (laptop asleep) counts at most the cap.
    sqlx::query("UPDATE playback_sessions SET last_heartbeat_at = now() - interval '10 minutes' WHERE id = $1")
        .bind(session)
        .execute(pool)
        .await
        .unwrap();
    playback::record_progress(pool, user, session, &update(2710.0, false))
        .await
        .unwrap()
        .unwrap();
    let (watched, stalls, stall_seconds, bandwidth, position): (f64, i32, f64, Option<i32>, f64) = sqlx::query_as(
        "SELECT watched_seconds, stall_count, stall_seconds, bandwidth_kbps, position_seconds FROM playback_sessions WHERE id = $1",
    )
    .bind(session)
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        (54.0..=56.0).contains(&watched),
        "20 s + 35 s cap, got {watched}"
    );
    assert_eq!(
        (stalls, stall_seconds, bandwidth, position),
        (2, 1.5, Some(24_000), 2710.0)
    );

    // Paused since the last report: no viewing time.
    sqlx::query("UPDATE playback_sessions SET last_heartbeat_at = now() - interval '25 seconds' WHERE id = $1")
        .bind(session)
        .execute(pool)
        .await
        .unwrap();
    playback::record_progress(pool, user, session, &update(2710.0, false))
        .await
        .unwrap();
    let (after,): (f64,) =
        sqlx::query_as("SELECT watched_seconds FROM playback_sessions WHERE id = $1")
            .bind(session)
            .fetch_one(pool)
            .await
            .unwrap();
    assert!((after - watched).abs() < 0.001);

    // Played to the end: watched, resume from the start, session closed.
    let mut end = update(5990.0, false);
    end.played_to_end = true;
    end.ends_session = true;
    let outcome = playback::record_progress(pool, user, session, &end)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        outcome,
        playback::ProgressOutcome {
            resume_position: 0.0,
            watched: true
        }
    );
    assert!(
        playback::record_progress(pool, user, session, &update(1.0, true))
            .await
            .unwrap()
            .is_none(),
        "ended"
    );

    // Someone else's session is not found.
    assert!(
        playback::record_progress(pool, Uuid::new_v4(), session, &update(1.0, true))
            .await
            .unwrap()
            .is_none()
    );
    let recent = playback::recent_sessions(pool, 10).await.unwrap();
    assert_eq!(
        (recent.len(), recent[0].username.as_str(), recent[0].active),
        (1, "viewer", false)
    );
    let list = playback::my_list(pool, user, &lisek_db::access::Restrictions::NONE).await.unwrap();
    assert_eq!(
        (list[0].season, list[0].episode, list[0].watched),
        (None, None, true)
    );
    db.drop().await;
}
