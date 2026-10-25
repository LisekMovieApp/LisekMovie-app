//! Database-backed tests for collections (box sets) and per-account
//! restrictions (parental controls).
//!
//! Opt-in: see `common/mod.rs`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lisek_db::access::{self, Restrictions};
use lisek_db::collections;
use lisek_db::{LibraryFilter, LibrarySort, PgPool};
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

/// Scans one movie and returns its id.
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
    let (id,): (Uuid,) = sqlx::query_as("SELECT id FROM media_items WHERE file_path = $1")
        .bind(&path)
        .fetch_one(pool)
        .await
        .unwrap();
    id
}

async fn rate(pool: &PgPool, item_id: Uuid, fields: serde_json::Value) {
    lisek_db::attach_provider_metadata(pool, item_id, "tmdb", &fields)
        .await
        .unwrap();
}

fn titles(items: &[lisek_core::MediaItem]) -> Vec<&str> {
    items.iter().map(|i| i.title.as_str()).collect()
}

async fn user(pool: &PgPool, name: &str, admin: bool) -> Uuid {
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (username, password_hash, is_admin) VALUES ($1, 'x', $2) RETURNING id",
    )
    .bind(name)
    .bind(admin)
    .fetch_one(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn collections_group_films_and_respect_restrictions() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let films = library(pool, "Filmy", "movies").await;
    let kids = library(pool, "Bajki", "movies").await;

    let new_hope = movie(pool, films, "A New Hope", 1977).await;
    let empire = movie(pool, films, "The Empire Strikes Back", 1980).await;
    let jedi = movie(pool, films, "Return of the Jedi", 1983).await;
    let alien = movie(pool, films, "Alien", 1979).await;
    let up = movie(pool, kids, "Up", 2009).await;
    rate(pool, new_hope, serde_json::json!({"age_rating": 10, "certification": "PG"})).await;
    rate(pool, empire, serde_json::json!({"age_rating": 13, "certification": "PG-13"})).await;
    rate(pool, alien, serde_json::json!({"age_rating": 17, "certification": "R"})).await;
    rate(pool, up, serde_json::json!({"age_rating": 0, "certification": "G"})).await;
    // Jedi stays unrated.

    let sw = collections::upsert(pool, "tmdb", "10", "Star Wars").await.unwrap();
    assert_eq!(sw.poster_url, None);
    let again = collections::upsert(pool, "tmdb", "10", "Star Wars Collection").await.unwrap();
    assert_eq!((again.id, again.name.as_str()), (sw.id, "Star Wars Collection"));
    for id in [new_hope, empire, jedi] {
        collections::link_item(pool, sw.id, id).await.unwrap();
    }
    collections::link_item(pool, sw.id, jedi).await.unwrap();
    assert_eq!(collections::of_item(pool, jedi).await.unwrap(), Some(sw.id));
    assert_eq!(collections::of_item(pool, alien).await.unwrap(), None);

    let all = Restrictions::NONE;
    let grouped = LibraryFilter {
        group_collections: true,
        ..LibraryFilter::default()
    };
    let page = lisek_db::list_items_by_library_page(pool, films, LibrarySort::TitleAsc, grouped, 10, 0, &all)
        .await
        .unwrap();
    assert_eq!(titles(&page), ["Alien", "Star Wars Collection"]);
    assert_eq!(page[1].item_type, "collection");
    assert_eq!(page[1].id, sw.id);
    assert_eq!(page[1].year, Some(1977), "the first film's year");
    assert_eq!(
        lisek_db::count_items_by_library(pool, films, grouped, &all).await.unwrap(),
        2
    );
    let recent = lisek_db::list_items_by_library_page(pool, films, LibrarySort::RecentlyAdded, grouped, 10, 0, &all)
        .await
        .unwrap();
    assert_eq!(titles(&recent), ["Alien", "Star Wars Collection"], "sorted by its newest film");
    let flat = lisek_db::list_items_by_library_page(pool, films, LibrarySort::TitleAsc, LibraryFilter::default(), 10, 0, &all)
        .await
        .unwrap();
    assert_eq!(flat.len(), 4);
    let searching = LibraryFilter {
        search: Some("e"),
        ..grouped
    };
    assert!(
        lisek_db::list_items_by_library_page(pool, films, LibrarySort::TitleAsc, searching, 10, 0, &all)
            .await
            .unwrap()
            .iter()
            .all(|i| i.item_type == "movie"),
        "a search lists films, not collections"
    );

    let cards = collections::library_collections(pool, films, &all, 10).await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!((cards[0].item_count, cards[0].first_year, cards[0].last_year), (3, Some(1977), Some(1983)));
    assert!(collections::library_collections(pool, kids, &all, 10).await.unwrap().is_empty());
    let members = collections::members(pool, sw.id, &all).await.unwrap();
    assert_eq!(
        titles(&members),
        ["A New Hope", "The Empire Strikes Back", "Return of the Jedi"],
        "release order"
    );
    let counts = collections::counts_in_library(pool, &[sw.id], films, &all).await.unwrap();
    assert_eq!(counts.get(&sw.id), Some(&3));

    // Up to 12: Empire (13) and Alien (17) disappear, unrated Jedi stays.
    let twelve = Restrictions {
        max_age_rating: Some(12),
        ..Restrictions::NONE
    };
    let page = lisek_db::list_items_by_library_page(pool, films, LibrarySort::TitleAsc, grouped, 10, 0, &twelve)
        .await
        .unwrap();
    assert_eq!(titles(&page), ["Star Wars Collection"]);
    assert_eq!(
        titles(&collections::members(pool, sw.id, &twelve).await.unwrap()),
        ["A New Hope", "Return of the Jedi"]
    );
    assert_eq!(collections::get(pool, sw.id, &twelve).await.unwrap().unwrap().item_count, 2);

    // Blocking unrated too leaves one film: no longer a group.
    let strict = Restrictions {
        block_unrated: true,
        ..twelve.clone()
    };
    let page = lisek_db::list_items_by_library_page(pool, films, LibrarySort::TitleAsc, grouped, 10, 0, &strict)
        .await
        .unwrap();
    assert_eq!(titles(&page), ["A New Hope"]);
    assert_eq!(page[0].item_type, "movie");
    assert_eq!(lisek_db::count_items_by_library(pool, films, grouped, &strict).await.unwrap(), 1);
    assert!(lisek_db::get_media_item(pool, alien, &strict).await.unwrap().is_none());
    assert_eq!(access::item_visible(pool, alien, &strict).await.unwrap(), Some(false));
    assert_eq!(access::item_visible(pool, new_hope, &strict).await.unwrap(), Some(true));
    assert_eq!(access::item_visible(pool, Uuid::new_v4(), &strict).await.unwrap(), None);
    assert_eq!(
        collections::artwork_stand_in(pool, sw.id, &strict).await.unwrap(),
        Some(new_hope)
    );

    // Only the kids' library.
    let kids_only = Restrictions {
        libraries: Some(vec![kids]),
        ..Restrictions::NONE
    };
    assert_eq!(
        titles(&lisek_db::list_recently_added(pool, None, 10, &kids_only).await.unwrap()),
        ["Up"]
    );
    assert_eq!(
        lisek_db::get_hero_item(pool, &kids_only).await.unwrap().map(|i| i.title),
        Some("Up".to_string())
    );
    assert!(collections::get(pool, sw.id, &kids_only).await.unwrap().is_none());
    assert_eq!(
        lisek_db::list_popular(pool, None, 10, &kids_only).await.unwrap().len(),
        1
    );
    assert_eq!(
        lisek_db::count_items_by_library(pool, films, LibraryFilter::default(), &kids_only)
            .await
            .unwrap(),
        0
    );

    // The rating column follows provider data; an admin's value wins.
    assert_eq!(access::item_rating(pool, empire).await.unwrap(), Some((Some(13), Some("PG-13".into()))));
    sqlx::query("UPDATE media_items SET metadata = metadata || '{\"age_rating\": 7}' WHERE id = $1")
        .bind(empire)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(access::item_rating(pool, empire).await.unwrap(), Some((Some(7), None)));
    assert_eq!(collections::members(pool, sw.id, &strict).await.unwrap().len(), 2);

    // A film the provider no longer puts in a collection leaves it; an
    // empty collection is pruned.
    for id in [new_hope, empire, jedi] {
        collections::unlink_item(pool, id, "tmdb").await.unwrap();
    }
    assert_eq!(collections::prune_empty(pool).await.unwrap(), vec![sw.id]);
    assert!(collections::all_ids(pool).await.unwrap().is_empty());
    db.drop().await;
}

#[tokio::test]
async fn restrictions_are_stored_normalized_and_never_bind_admins() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let films = library(pool, "Filmy", "movies").await;
    let kids = library(pool, "Bajki", "movies").await;
    let child = user(pool, "dziecko", false).await;
    let admin = user(pool, "admin", true).await;

    assert_eq!(access::get_restrictions(pool, child).await.unwrap(), Some((false, Restrictions::NONE)));
    assert_eq!(access::get_restrictions(pool, Uuid::new_v4()).await.unwrap(), None);
    assert!(!access::set_restrictions(pool, Uuid::new_v4(), &Restrictions::NONE).await.unwrap());

    let wanted = Restrictions {
        libraries: Some(vec![kids, kids]),
        max_age_rating: Some(7),
        block_unrated: true,
    };
    assert!(access::set_restrictions(pool, child, &wanted).await.unwrap());
    assert!(access::set_restrictions(pool, admin, &wanted).await.unwrap());
    let (is_admin, stored) = access::get_restrictions(pool, child).await.unwrap().unwrap();
    assert!(!is_admin);
    assert_eq!(stored.libraries, Some(vec![kids]));
    assert_eq!((stored.max_age_rating, stored.block_unrated), (Some(7), true));
    let listed = access::list_restrictions(pool).await.unwrap();
    assert_eq!(listed.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![child], "admins are never listed");

    for (id, token) in [(child, "child-token"), (admin, "admin-token")] {
        lisek_db::create_session(pool, id, token, "test", "test", "test", chrono::Utc::now() + chrono::Duration::hours(1))
            .await
            .unwrap();
    }
    let (user, restrictions) = access::find_session(pool, "child-token").await.unwrap().unwrap();
    assert_eq!(user.id, child);
    assert!(restrictions.allows_library(kids) && !restrictions.allows_library(films));
    let (_, admin_access) = access::find_session(pool, "admin-token").await.unwrap().unwrap();
    assert_eq!(admin_access, Restrictions::NONE, "administrators see everything");
    assert!(access::find_session(pool, "nope").await.unwrap().is_none());

    // My list hides what the account may no longer see.
    let film = movie(pool, films, "Heat", 1995).await;
    lisek_db::playback::set_flags(pool, child, film, None, Some(true)).await.unwrap();
    assert!(lisek_db::playback::my_list(pool, child, &restrictions).await.unwrap().is_empty());
    assert_eq!(lisek_db::playback::my_list(pool, child, &Restrictions::NONE).await.unwrap().len(), 1);

    // Unrestricted again: the row goes away; block_unrated alone means nothing.
    let loose = Restrictions {
        block_unrated: true,
        ..Restrictions::NONE
    };
    assert!(access::set_restrictions(pool, child, &loose).await.unwrap());
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM user_restrictions WHERE user_id = $1")
        .bind(child)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    db.drop().await;
}

#[tokio::test]
async fn series_cards_use_their_episodes_rating() {
    let Some(db) = TestDb::create().await else { return };
    let pool = &db.pool;
    let tv = library(pool, "Seriale", "tv_shows").await;
    lisek_db::persist_scan_results(
        pool,
        tv,
        &[ScanResult::Episode(ScannedEpisode {
            file_path: PathBuf::from("/media/Seriale/BB/S01E01.mkv"),
            series_title: "Breaking Bad".into(),
            season: 1,
            episode: 1,
            end_episode: None,
            file: facts(),
        })],
    )
    .await
    .unwrap();
    let (episode,): (Uuid,) = sqlx::query_as("SELECT id FROM media_items")
        .fetch_one(pool)
        .await
        .unwrap();
    rate(pool, episode, serde_json::json!({"age_rating": 17})).await;
    let teen = Restrictions {
        max_age_rating: Some(16),
        ..Restrictions::NONE
    };
    assert!(lisek_db::list_recently_added(pool, Some(tv), 10, &teen).await.unwrap().is_empty());
    assert!(lisek_db::get_media_item(pool, episode, &teen).await.unwrap().is_none());
    assert_eq!(
        lisek_db::list_recently_added(pool, Some(tv), 10, &Restrictions::NONE).await.unwrap()[0].item_type,
        "series"
    );
    db.drop().await;
}
