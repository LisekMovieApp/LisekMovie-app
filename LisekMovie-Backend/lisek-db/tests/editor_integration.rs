//! Database-backed tests for the Faza 20 metadata editor: series renamed in
//! place, episode fields, edits surviving rescans and provider refreshes,
//! manual artwork. Opt-in: see `common/mod.rs`.

use std::path::PathBuf;

use lisek_db::editor::{self, EditError, EpisodeFields, SharedFields};
use lisek_db::PgPool;
use lisek_library::{FileFacts, ScanResult, ScannedEpisode, ScannedMovie};
use uuid::Uuid;

mod common;
use common::TestDb;

fn facts() -> FileFacts {
    FileFacts { size_bytes: Some(1), modified: None }
}

fn episode(path: &str, show: &str, season: i32, number: i32) -> ScanResult {
    ScanResult::Episode(ScannedEpisode {
        file_path: PathBuf::from(path),
        series_title: show.into(),
        season,
        episode: number,
        end_episode: None,
        file: facts(),
    })
}

async fn id_of(pool: &PgPool, path: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM media_items WHERE file_path = $1").bind(path).fetch_one(pool).await.unwrap()
}

fn shared<'a>(title: &'a str, locks: &'a [String]) -> SharedFields<'a> {
    SharedFields {
        title,
        original_title: None,
        year: Some(2020),
        overview: Some("Opis serialu"),
        metadata_locked: false,
        forced_provider: None,
        locked_fields: locks,
        age_rating: None,
        rating: None,
        directors: &[],
    }
}

#[tokio::test]
async fn series_rename_keeps_the_series_and_survives_a_rescan() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = lisek_db::create_library(pool, "Seriale", "tv_shows", &["/tv".into()], false).await.unwrap().id;
    let scan = vec![
        episode("/tv/Rojbers/Season 01/Rojbers S01E01.mkv", "Rojbers", 1, 1),
        episode("/tv/Rojbers/Season 01/Rojbers S01E02.mkv", "Rojbers", 1, 2),
        episode("/tv/Inny/Season 01/Inny S01E01.mkv", "Inny", 1, 1),
    ];
    lisek_db::persist_scan_results(pool, lib, &scan).await.unwrap();
    let first = id_of(pool, "/tv/Rojbers/Season 01/Rojbers S01E01.mkv").await;
    let series_before: Uuid = sqlx::query_scalar("SELECT series_id FROM episodes WHERE media_item_id = $1")
        .bind(first).fetch_one(pool).await.unwrap();

    // The other series' name is taken.
    let locks = vec!["title".to_string()];
    assert!(matches!(editor::update_series(pool, first, &shared("Inny", &locks)).await, Err(EditError::TitleTaken)));

    let updated = editor::update_series(pool, first, &shared("Rojbersi", &locks)).await.unwrap();
    assert_eq!(updated, 2);
    let (series_after, title, count): (Uuid, String, i32) = sqlx::query_as(
        "SELECT s.id, s.title, s.episode_count FROM episodes e JOIN series s ON s.id = e.series_id WHERE e.media_item_id = $1",
    ).bind(first).fetch_one(pool).await.unwrap();
    assert_eq!((series_after, title.as_str(), count), (series_before, "Rojbersi", 2));
    let (overview, locked): (Option<String>, Vec<String>) = sqlx::query_as(
        "SELECT overview, locked_fields FROM media_items WHERE file_path LIKE '/tv/Rojbers/%' LIMIT 1",
    ).fetch_one(pool).await.unwrap();
    assert_eq!(overview.as_deref(), Some("Opis serialu"));
    assert!(locked.contains(&"title".to_string()));

    // A rescan takes titles from the folder name, but not locked ones.
    lisek_db::persist_scan_results(pool, lib, &scan).await.unwrap();
    let titles: Vec<String> = sqlx::query_scalar("SELECT DISTINCT title FROM media_items WHERE file_path LIKE '/tv/Rojbers/%'")
        .fetch_all(pool).await.unwrap();
    assert_eq!(titles, vec!["Rojbersi".to_string()]);
    let series_count: i64 = sqlx::query_scalar("SELECT count(*) FROM series").fetch_one(pool).await.unwrap();
    assert_eq!(series_count, 2);
    db.drop().await;
}

#[tokio::test]
async fn episode_fields_survive_a_provider_refresh() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = lisek_db::create_library(pool, "Seriale", "tv_shows", &["/tv".into()], false).await.unwrap().id;
    lisek_db::persist_scan_results(pool, lib, &[episode("/tv/A/Season 01/A S01E01.mkv", "A", 1, 1)]).await.unwrap();
    let id = id_of(pool, "/tv/A/Season 01/A S01E01.mkv").await;
    lisek_db::attach_provider_metadata(pool, id, "tmdb", &serde_json::json!({
        "overview": "series", "episode": {"title": "Pilot", "overview": "Provider", "still_url": "s1"}
    })).await.unwrap();

    let ctx = editor::episode_context(pool, id).await.unwrap();
    assert_eq!(ctx.episode_title.as_deref(), Some("Pilot"));
    assert_eq!(ctx.series_title.as_deref(), Some("A"));

    let locks = vec!["episode_title".to_string(), "episode_rating".to_string(), "episode_directors".to_string()];
    let directors = vec!["Jan Reżyser".to_string()];
    editor::update_episode(pool, id, &EpisodeFields {
        title: Some("Najwyższy czas"), overview: Some("Mój opis"), air_date: Some("2026-01-02"),
        rating: Some(7.5), directors: &directors, locked_fields: &locks,
    }).await.unwrap();
    // The provider comes back with its own episode data.
    lisek_db::attach_provider_metadata(pool, id, "tmdb", &serde_json::json!({
        "overview": "series 2", "episode": {"title": "Pilot", "overview": "Provider 2", "still_url": "s2", "rating": 5.0}
    })).await.unwrap();
    let ctx = editor::episode_context(pool, id).await.unwrap();
    assert_eq!(ctx.episode_title.as_deref(), Some("Najwyższy czas"), "locked title kept");
    assert_eq!(ctx.episode_overview.as_deref(), Some("Provider 2"), "unlocked overview refreshed");
    assert_eq!(ctx.episode_rating, Some(7.5), "locked score kept");
    assert_eq!(ctx.episode_directors, directors, "locked directors kept");
    let title: String = sqlx::query_scalar("SELECT title FROM media_items WHERE id = $1").bind(id).fetch_one(pool).await.unwrap();
    assert_eq!(title, "A", "the episode stays in its series");

    // A film is not an episode.
    let movie = ScanResult::Movie(ScannedMovie { file_path: "/m/F (2020).mkv".into(), title: "F".into(), year: Some(2020), file: facts() });
    let films = lisek_db::create_library(pool, "Filmy", "movies", &["/m".into()], false).await.unwrap().id;
    lisek_db::persist_scan_results(pool, films, &[movie]).await.unwrap();
    let film = id_of(pool, "/m/F (2020).mkv").await;
    let none: Vec<String> = vec![];
    assert!(matches!(
        editor::update_episode(pool, film, &EpisodeFields { title: None, overview: None, air_date: None, rating: None, directors: &[], locked_fields: &none }).await,
        Err(EditError::NotAnEpisode)
    ));
    assert!(matches!(editor::update_series(pool, film, &shared("x", &none)).await, Err(EditError::NotAnEpisode)));
    assert!(matches!(editor::update_item(pool, Uuid::new_v4(), &shared("x", &none)).await, Err(EditError::NotFound)));
    db.drop().await;
}

#[tokio::test]
async fn manual_artwork_is_locked_for_the_whole_series() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = lisek_db::create_library(pool, "Seriale", "tv_shows", &["/tv".into()], false).await.unwrap().id;
    lisek_db::persist_scan_results(pool, lib, &[
        episode("/tv/B/Season 01/B S01E01.mkv", "B", 1, 1),
        episode("/tv/B/Season 01/B S01E02.mkv", "B", 1, 2),
    ]).await.unwrap();
    let one = id_of(pool, "/tv/B/Season 01/B S01E01.mkv").await;
    let two = id_of(pool, "/tv/B/Season 01/B S01E02.mkv").await;

    editor::record_artwork_override(pool, &[one], "poster", 5_000).await.unwrap();
    let (_, since) = editor::artwork_source(pool, one, "poster").await.unwrap().unwrap();
    assert_eq!(since, Some(5_000));
    let (_, other_kind) = editor::artwork_source(pool, one, "backdrop").await.unwrap().unwrap();
    assert_eq!(other_kind, None);
    // A sibling (e.g. a newly scanned episode) honours the series' lock.
    assert!(editor::effective_locked_fields(pool, two).await.unwrap().contains(&"poster".to_string()));
    // Episode-level locks stay with their episode.
    editor::record_artwork_override(pool, &[one], "thumb", 6_000).await.unwrap();
    assert!(!editor::effective_locked_fields(pool, two).await.unwrap().contains(&"thumb".to_string()));
    assert!(editor::effective_locked_fields(pool, one).await.unwrap().contains(&"thumb".to_string()));
    db.drop().await;
}

#[tokio::test]
async fn season_edits_and_series_directors_survive_a_refresh_and_a_rescan() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let lib = lisek_db::create_library(pool, "Seriale", "tv_shows", &["/tv".into()], false).await.unwrap().id;
    let scan = vec![
        episode("/tv/S/Season 01/S S01E01.mkv", "S", 1, 1),
        episode("/tv/S/Season 01/S S01E02.mkv", "S", 1, 2),
    ];
    lisek_db::persist_scan_results(pool, lib, &scan).await.unwrap();
    let first = id_of(pool, "/tv/S/Season 01/S S01E01.mkv").await;

    let season = editor::season_detail(pool, first, 1).await.unwrap().unwrap();
    assert_eq!((season.episode_count, season.name.as_deref()), (2, None));
    assert!(editor::season_detail(pool, first, 7).await.unwrap().is_none());
    let locks = vec!["name".to_string(), "overview".to_string(), "bogus".to_string()];
    editor::update_season(pool, season.series_id, 1, &editor::SeasonFields {
        name: Some("Początek"), overview: Some("Mój sezon"), air_year: Some(2024), locked_fields: &locks,
    }).await.unwrap();
    // The provider's season data: locked name and overview stay, the year is not locked.
    lisek_db::seasons::set_season_metadata(pool, season.series_id, 1, Some("Season 1"), Some("Provider"), Some(2019))
        .await.unwrap();
    let after = editor::season_detail(pool, first, 1).await.unwrap().unwrap();
    assert_eq!((after.name.as_deref(), after.overview.as_deref(), after.air_year), (Some("Początek"), Some("Mój sezon"), Some(2019)));
    assert_eq!(after.locked_fields, vec!["name".to_string(), "overview".to_string()]);

    // Series-wide directors typed by hand stay through a rescan.
    let directors = vec!["Reżyserka".to_string()];
    let title_lock = vec!["title".to_string(), "directors".to_string()];
    let mut fields = shared("S", &title_lock);
    fields.directors = &directors;
    fields.rating = Some(8.0);
    editor::update_series(pool, first, &fields).await.unwrap();
    lisek_db::persist_scan_results(pool, lib, &scan).await.unwrap();
    let ctx = editor::episode_context(pool, id_of(pool, "/tv/S/Season 01/S S01E02.mkv").await).await.unwrap();
    assert_eq!((ctx.directors, ctx.rating), (directors, Some(8.0)));
    let kept = editor::season_detail(pool, first, 1).await.unwrap().unwrap();
    assert_eq!(kept.name.as_deref(), Some("Początek"), "the season row survives a rescan");
    db.drop().await;
}
