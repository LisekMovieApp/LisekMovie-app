//! Database-backed tests for the alpha picker's jump table (Faza 12):
//! `library_letter_index` must point at exactly the card the paged list
//! shows at that position, for both title sorts and with collection
//! grouping.
//!
//! Opt-in: see `common/mod.rs`.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use lisek_db::access::Restrictions;
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

fn movie(i: usize, title: &str) -> ScanResult {
    ScanResult::Movie(ScannedMovie {
        file_path: PathBuf::from(format!("/media/m{i}.mkv")),
        title: title.into(),
        year: Some(2000 + i as i32),
        file: facts(),
    })
}

/// The picker's letter for a title, as the SQL folds it.
fn folded(title: &str) -> String {
    let first = title.chars().next().unwrap().to_uppercase().next().unwrap();
    let base = match first {
        'Ł' => 'L',
        'É' => 'E',
        'Ś' => 'S',
        other => other,
    };
    if base.is_ascii_uppercase() {
        base.to_string()
    } else {
        "#".to_string()
    }
}

async fn all_titles(pool: &PgPool, library: Uuid, sort: LibrarySort, grouped: bool) -> Vec<String> {
    let filter = LibraryFilter {
        group_collections: grouped,
        ..LibraryFilter::default()
    };
    lisek_db::list_items_by_library_page(pool, library, sort, filter, 200, 0, &Restrictions::NONE)
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.title)
        .collect()
}

/// Every returned letter points at that letter's first card, letters come
/// in list order, and no letter with cards is missing.
async fn check(
    pool: &PgPool,
    library: Uuid,
    sort: LibrarySort,
    grouped: bool,
) -> Vec<(String, i64)> {
    let titles = all_titles(pool, library, sort, grouped).await;
    let index = lisek_db::library_letter_index(pool, library, sort, grouped, &Restrictions::NONE)
        .await
        .unwrap();
    let mut expected: Vec<(String, i64)> = Vec::new();
    for (i, title) in titles.iter().enumerate() {
        let letter = folded(title);
        if !expected.iter().any(|(l, _)| *l == letter) {
            expected.push((letter, i as i64));
        }
    }
    let got: Vec<(String, i64)> = index.into_iter().map(|l| (l.letter, l.index)).collect();
    assert_eq!(got, expected, "{sort:?} grouped={grouped}: {titles:?}");
    got
}

#[tokio::test]
async fn letter_index_matches_the_paged_list() {
    let Some(db) = TestDb::create().await else {
        return;
    };
    let pool = &db.pool;
    let films = lisek_db::create_library(pool, "Filmy", "movies", &["/media".to_string()], false)
        .await
        .unwrap()
        .id;
    let titles = [
        "2001: A Space Odyssey",
        "Alien",
        "aliens",
        "Matrix",
        "Memento",
        "Łowcy smoków",
        "Émilie",
        "Śnieżka",
        "Zodiac",
        "A New Hope",
        "Return of the Jedi",
    ];
    let scan: Vec<_> = titles
        .iter()
        .enumerate()
        .map(|(i, t)| movie(i, t))
        .collect();
    lisek_db::persist_scan_results(pool, films, &scan)
        .await
        .unwrap();
    // A series in the same library counts as one card.
    lisek_db::persist_scan_results(
        pool,
        films,
        &[ScanResult::Episode(ScannedEpisode {
            file_path: PathBuf::from("/media/show/S01E01.mkv"),
            series_title: "Mandalorian".into(),
            season: 1,
            episode: 1,
            end_episode: None,
            file: facts(),
        })],
    )
    .await
    .unwrap();

    let asc = check(pool, films, LibrarySort::TitleAsc, false).await;
    let letters: Vec<&str> = asc.iter().map(|(l, _)| l.as_str()).collect();
    for wanted in ["#", "A", "E", "L", "M", "R", "S", "Z"] {
        assert!(
            letters.contains(&wanted),
            "{wanted} missing from {letters:?}"
        );
    }
    check(pool, films, LibrarySort::TitleDesc, false).await;

    // Two films of one collection fold into a "Star Wars" card: A New Hope
    // leaves A's count, S gains the collection card.
    let sw = collections::upsert(pool, "tmdb", "10", "Star Wars")
        .await
        .unwrap();
    for title in ["A New Hope", "Return of the Jedi"] {
        let id: Uuid = sqlx::query_scalar("SELECT id FROM media_items WHERE title = $1")
            .bind(title)
            .fetch_one(pool)
            .await
            .unwrap();
        collections::link_item(pool, sw.id, id).await.unwrap();
    }
    let grouped = check(pool, films, LibrarySort::TitleAsc, true).await;
    assert!(
        !grouped.iter().any(|(l, _)| l == "R"),
        "Return of the Jedi is inside the collection"
    );
    check(pool, films, LibrarySort::TitleDesc, true).await;

    // Restrictions: another account sees nothing in a library it may not open.
    let none = Restrictions {
        libraries: Some(vec![Uuid::new_v4()]),
        ..Restrictions::NONE
    };
    assert!(
        lisek_db::library_letter_index(pool, films, LibrarySort::TitleAsc, false, &none)
            .await
            .unwrap()
            .is_empty()
    );
    db.drop().await;
}
