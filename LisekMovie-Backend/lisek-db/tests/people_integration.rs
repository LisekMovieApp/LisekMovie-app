//! Database-backed tests for cast and crew (`people`, `media_people`).
//!
//! Opt-in: see `common/mod.rs`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lisek_db::PgPool;
use lisek_db::access::Restrictions;
use lisek_db::people::{self, NewCredit, PersonDetails};
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

async fn library(pool: &PgPool, name: &str, kind: &str) -> Uuid {
    lisek_db::create_library(pool, name, kind, &[format!("/media/{name}")], false)
        .await
        .unwrap()
        .id
}

async fn id_of(pool: &PgPool, path: &str) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as("SELECT id FROM media_items WHERE file_path = $1")
        .bind(path)
        .fetch_one(pool)
        .await
        .unwrap();
    id
}

async fn movie(pool: &PgPool, library_id: Uuid, title: &str, year: i32) -> Uuid {
    let path = format!("/media/{library_id}/{title}.mkv");
    lisek_db::persist_scan_results(
        pool,
        library_id,
        &[ScanResult::Movie(ScannedMovie {
            file_path: PathBuf::from(&path),
            title: title.into(),
            year: Some(year),
            file: facts(),
        })],
    )
    .await
    .unwrap();
    id_of(pool, &path).await
}

fn credit(tmdb_id: i64, name: &str, role: &str, character: Option<&str>, order: i32) -> NewCredit {
    NewCredit {
        tmdb_id,
        name: name.into(),
        role: role.into(),
        character: character.map(str::to_string),
        order,
    }
}

async fn credit_rows(pool: &PgPool) -> Vec<(String, String, Option<String>, i32)> {
    sqlx::query_as(
        "SELECT p.name, mp.role, mp.character_name, mp.sort_order FROM media_people mp
         JOIN people p ON p.id = mp.person_id ORDER BY p.name, mp.role",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn credits_link_people_across_titles_and_only_change_what_changed() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let films = library(pool, "Filmy", "movies").await;
    let hope = movie(pool, films, "A New Hope", 1977).await;
    let raiders = movie(pool, films, "Raiders of the Lost Ark", 1981).await;

    let linked = people::replace_credits(
        pool,
        hope,
        &[
            credit(2, "Mark Hamill", "Actor", Some("Luke Skywalker"), 0),
            credit(3, "Harrison Ford", "Actor", Some("Han Solo"), 1),
            credit(1, "George Lucas", "Director", None, 0),
            credit(1, "George Lucas", "Writer", None, 0),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        linked.iter().map(|p| p.tmdb_id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let ford = linked[2].id;
    people::replace_credits(
        pool,
        raiders,
        &[credit(
            3,
            "Harrison Ford",
            "Actor",
            Some("Indiana Jones"),
            0,
        )],
    )
    .await
    .unwrap();
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM people")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 3, "one row per person");

    let credits = people::item_credits(pool, hope, 50).await.unwrap();
    assert_eq!(
        credits
            .iter()
            .map(|c| (c.name.as_str(), c.role.as_str()))
            .collect::<Vec<_>>(),
        [
            ("George Lucas", "Director"),
            ("George Lucas", "Writer"),
            ("Mark Hamill", "Actor"),
            ("Harrison Ford", "Actor"),
        ]
    );
    assert!(credits.iter().all(|c| !c.has_image));

    // A refresh with the same cast keeps the rows as they are (no update).
    let same = [
        credit(2, "Mark Hamill", "Actor", Some("Luke Skywalker"), 0),
        credit(3, "Harrison Ford", "Actor", Some("Han Solo"), 1),
        credit(1, "George Lucas", "Director", None, 0),
        credit(1, "George Lucas", "Writer", None, 0),
    ];
    let xmin = |pool: &PgPool| {
        let pool = pool.clone();
        async move {
            let rows: Vec<(String,)> =
                sqlx::query_as("SELECT xmin::text FROM media_people ORDER BY person_id, role")
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            rows
        }
    };
    let before = xmin(pool).await;
    people::replace_credits(pool, hope, &same).await.unwrap();
    assert_eq!(
        xmin(pool).await,
        before,
        "unchanged credits are not rewritten"
    );
    assert_eq!(credit_rows(pool).await.len(), 5);

    // A changed part and a dropped credit.
    people::replace_credits(
        pool,
        hope,
        &[
            credit(2, "Mark Hamill", "Actor", Some("Luke"), 0),
            credit(1, "George Lucas", "Director", None, 0),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        credit_rows(pool).await,
        [
            ("George Lucas".to_string(), "Director".to_string(), None, 0),
            (
                "Harrison Ford".to_string(),
                "Actor".to_string(),
                Some("Indiana Jones".to_string()),
                0
            ),
            (
                "Mark Hamill".to_string(),
                "Actor".to_string(),
                Some("Luke".to_string()),
                0
            ),
        ]
    );

    // Ford is still credited on Raiders: no orphans yet, then one.
    assert!(people::prune_orphans(pool).await.unwrap().is_empty());
    people::replace_credits(pool, raiders, &[]).await.unwrap();
    assert_eq!(people::prune_orphans(pool).await.unwrap(), [ford]);

    db.drop().await;
}

#[tokio::test]
async fn a_person_page_respects_restrictions_and_folds_series_episodes() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let films = library(pool, "Filmy", "movies").await;
    let shows = library(pool, "Seriale", "tv_shows").await;
    let drive = movie(pool, films, "Drive", 2011).await;
    lisek_db::persist_scan_results(
        pool,
        shows,
        &(1..=3)
            .map(|n| {
                ScanResult::Episode(ScannedEpisode {
                    file_path: PathBuf::from(format!("/media/Seriale/BB/S01E0{n}.mkv")),
                    series_title: "Breaking Bad".into(),
                    season: 1,
                    episode: n,
                    end_episode: None,
                    file: facts(),
                })
            })
            .collect::<Vec<_>>(),
    )
    .await
    .unwrap();
    let mut episodes = Vec::new();
    for n in 1..=3 {
        episodes.push(id_of(pool, &format!("/media/Seriale/BB/S01E0{n}.mkv")).await);
    }
    let cranston = credit(17419, "Bryan Cranston", "Actor", Some("Walter White"), 0);
    for episode in &episodes {
        people::replace_credits(pool, *episode, std::slice::from_ref(&cranston))
            .await
            .unwrap();
    }
    let person = people::replace_credits(
        pool,
        drive,
        &[credit(17419, "Bryan Cranston", "Actor", Some("Shannon"), 2)],
    )
    .await
    .unwrap()[0]
        .id;

    let all = Restrictions::NONE;
    let found = people::get_visible(pool, person, &all)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (found.name.as_str(), found.tmdb_id),
        ("Bryan Cranston", 17419)
    );
    assert_eq!(found.details_updated_at, None);
    let titles = people::filmography(pool, person, &all, 100).await.unwrap();
    assert_eq!(
        titles
            .iter()
            .map(|t| (
                t.item.title.as_str(),
                t.item.item_type.as_str(),
                t.character.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("Drive", "movie", Some("Shannon")),
            ("Breaking Bad", "series", Some("Walter White"))
        ],
        "dated films first, one card per series"
    );
    assert_eq!(titles[1].roles, ["Actor"]);
    assert!(
        episodes.contains(&titles[1].item.id),
        "the series card is an episode id"
    );

    // Only the films library: the series disappears; no library: the person too.
    let films_only = Restrictions {
        libraries: Some(vec![films]),
        ..Restrictions::NONE
    };
    let titles = people::filmography(pool, person, &films_only, 100)
        .await
        .unwrap();
    assert_eq!(titles.len(), 1);
    let nothing = Restrictions {
        libraries: Some(vec![]),
        ..Restrictions::NONE
    };
    assert_eq!(
        people::get_visible(pool, person, &nothing).await.unwrap(),
        None
    );
    assert!(
        people::filmography(pool, person, &nothing, 100)
            .await
            .unwrap()
            .is_empty()
    );

    // Details from the provider; an empty name keeps the stored one.
    let details = PersonDetails {
        name: Some(" ".into()),
        biography: Some("An American actor.".into()),
        birth_place: Some("Hollywood, California, USA".into()),
        birth_date: chrono::NaiveDate::from_ymd_opt(1956, 3, 7),
        death_date: None,
        imdb_id: Some("nm0186505".into()),
    };
    people::save_details(pool, person, Some(&details))
        .await
        .unwrap();
    let found = people::get_visible(pool, person, &all)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.name, "Bryan Cranston");
    assert_eq!(
        found.birth_date,
        chrono::NaiveDate::from_ymd_opt(1956, 3, 7)
    );
    assert_eq!(found.imdb_id.as_deref(), Some("nm0186505"));
    assert!(found.details_updated_at.is_some());

    people::set_profile_url(pool, person, Some("https://image.tmdb.org/t/p/w185/bc.jpg"))
        .await
        .unwrap();
    assert!(people::item_credits(pool, drive, 10).await.unwrap()[0].has_image);

    // Deleting the films removes their credits with them.
    sqlx::query("DELETE FROM media_items")
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(people::prune_orphans(pool).await.unwrap(), [person]);

    db.drop().await;
}
