//! Browse side: home feed, library pages, series detail.
//!
//! Every card is either a movie (`media_items` row) or a series (`series`
//! row, migration 0013). A series card is addressed by its representative
//! episode's id — the most recently added one — so poster/backdrop URLs
//! and `/media/{id}` links keep the shape they had before series became a
//! table.
//!
//! Each list query is a `UNION ALL` of two independently sorted, LIMITed
//! index scans (movies, series) merged by an outer sort of at most
//! `2 * limit` rows. Branch and outer ORDER BY use the same keys, so the
//! per-branch LIMIT never drops a row the merged result needs.
//!
//! Static queries use `query_as!`, so a typo in a column name or a type
//! mismatch fails `cargo build`, not a request. The "!"/"?" suffixes on
//! output columns pin nullability where Postgres cannot infer it through
//! `UNION ALL`.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::AssertSqlSafe;
use uuid::Uuid;

use lisek_core::MediaItem;

use crate::{DbError, PgPool, access::Restrictions};

/// One card as the catalog queries return it. Converted into the public
/// [`MediaItem`] at the edge of this module.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct CatalogRow {
    id: Uuid,
    library_id: Uuid,
    item_type: String,
    title: String,
    year: Option<i32>,
    overview: Option<String>,
    season: Option<i32>,
    episode: Option<i32>,
    created_at: DateTime<Utc>,
}

impl From<CatalogRow> for MediaItem {
    fn from(row: CatalogRow) -> Self {
        MediaItem {
            id: row.id,
            library_id: row.library_id,
            item_type: row.item_type,
            title: row.title,
            year: row.year,
            overview: row.overview,
            season: row.season,
            episode: row.episode,
            created_at: row.created_at,
        }
    }
}

pub(crate) fn into_items(rows: Vec<CatalogRow>) -> Vec<MediaItem> {
    rows.into_iter().map(MediaItem::from).collect()
}

/// The single item the Hero Banner features: preferring one a metadata
/// provider has actually matched (so there's a real synopsis to show)
/// over just whatever was scanned most recently, but falling back to plain
/// scan recency once nothing is enriched yet (e.g. right after a library's
/// very first scan, before any provider is configured) rather than leaving
/// the banner empty. A show can feature here exactly like a movie can.
pub async fn get_hero_item(
    pool: &PgPool,
    access: &Restrictions,
) -> Result<Option<MediaItem>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    let row = sqlx::query_as!(
        CatalogRow,
        r#"
        SELECT c.id AS "id!", c.library_id AS "library_id!", c.item_type AS "item_type!",
               c.title AS "title!", c.year AS "year?", c.overview AS "overview?",
               c.season AS "season?", c.episode AS "episode?", c.created_at AS "created_at!"
        FROM (
            (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                    NULL::int AS season, NULL::int AS episode, m.created_at
             FROM media_items m
             WHERE m.item_type = 'movie' AND m.overview IS NOT NULL
               AND lisek_visible(m.library_id, m.age_rating, $1, $2, $3)
             ORDER BY m.created_at DESC, m.id DESC
             LIMIT 1)
            UNION ALL
            (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int, r.overview,
                    NULL::int, NULL::int, s.last_episode_added_at
             FROM series s
             JOIN media_items r ON r.id = s.representative_item_id
             WHERE r.overview IS NOT NULL AND lisek_visible(s.library_id, r.age_rating, $1, $2, $3)
             ORDER BY s.last_episode_added_at DESC, s.representative_item_id DESC
             LIMIT 1)
            UNION ALL
            (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                    NULL::int, NULL::int, m.created_at
             FROM media_items m
             WHERE m.item_type = 'movie' AND lisek_visible(m.library_id, m.age_rating, $1, $2, $3)
             ORDER BY m.created_at DESC, m.id DESC
             LIMIT 1)
            UNION ALL
            (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int, r.overview,
                    NULL::int, NULL::int, s.last_episode_added_at
             FROM series s
             JOIN media_items r ON r.id = s.representative_item_id
             WHERE lisek_visible(s.library_id, r.age_rating, $1, $2, $3)
             ORDER BY s.last_episode_added_at DESC, s.representative_item_id DESC
             LIMIT 1)
        ) c
        ORDER BY (c.overview IS NOT NULL) DESC, c.created_at DESC, c.id DESC
        LIMIT 1
        "#,
        libraries,
        max_age,
        block_unrated
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(MediaItem::from))
}

/// The most recently scanned items, newest first. A series sorts by its
/// newest episode, so a new episode bumps a show back to the top exactly
/// like a freshly scanned movie. `library_id: None` is the catalog-wide
/// home row; `Some(id)` backs `/library/[id]`'s "recently added" row.
/// Each variant is its own literal so each gets the matching index.
pub async fn list_recently_added(
    pool: &PgPool,
    library_id: Option<Uuid>,
    limit: i64,
    access: &Restrictions,
) -> Result<Vec<MediaItem>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    let rows = match library_id {
        None => {
            sqlx::query_as!(
                CatalogRow,
                r#"
                SELECT c.id AS "id!", c.library_id AS "library_id!", c.item_type AS "item_type!",
                       c.title AS "title!", c.year AS "year?", c.overview AS "overview?",
                       c.season AS "season?", c.episode AS "episode?", c.created_at AS "created_at!"
                FROM (
                    (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                            NULL::int AS season, NULL::int AS episode, m.created_at
                     FROM media_items m
                     WHERE m.item_type = 'movie' AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
                     ORDER BY m.created_at DESC, m.id DESC
                     LIMIT $1)
                    UNION ALL
                    (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int,
                            r.overview, NULL::int, NULL::int, s.last_episode_added_at
                     FROM series s
                     JOIN media_items r ON r.id = s.representative_item_id
                     WHERE lisek_visible(s.library_id, r.age_rating, $2, $3, $4)
                     ORDER BY s.last_episode_added_at DESC, s.representative_item_id DESC
                     LIMIT $1)
                ) c
                ORDER BY c.created_at DESC, c.id DESC
                LIMIT $1
                "#,
                limit,
                libraries,
                max_age,
                block_unrated
            )
            .fetch_all(pool)
            .await?
        }
        Some(library_id) => {
            sqlx::query_as!(
                CatalogRow,
                r#"
                SELECT c.id AS "id!", c.library_id AS "library_id!", c.item_type AS "item_type!",
                       c.title AS "title!", c.year AS "year?", c.overview AS "overview?",
                       c.season AS "season?", c.episode AS "episode?", c.created_at AS "created_at!"
                FROM (
                    (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                            NULL::int AS season, NULL::int AS episode, m.created_at
                     FROM media_items m
                     WHERE m.item_type = 'movie' AND m.library_id = $2
                       AND lisek_visible(m.library_id, m.age_rating, $3, $4, $5)
                     ORDER BY m.created_at DESC, m.id DESC
                     LIMIT $1)
                    UNION ALL
                    (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int,
                            r.overview, NULL::int, NULL::int, s.last_episode_added_at
                     FROM series s
                     JOIN media_items r ON r.id = s.representative_item_id
                     WHERE s.library_id = $2 AND lisek_visible(s.library_id, r.age_rating, $3, $4, $5)
                     ORDER BY s.last_episode_added_at DESC, s.representative_item_id DESC
                     LIMIT $1)
                ) c
                ORDER BY c.created_at DESC, c.id DESC
                LIMIT $1
                "#,
                limit,
                library_id,
                libraries,
                max_age,
                block_unrated
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(into_items(rows))
}

/// "Top 10 Today"'s backing query. There's no watch-count or trending
/// signal anywhere in this schema yet — rather than fake a ranking, this
/// orders by release year (then scan recency), which reads honestly as
/// "the newest releases in your library". A series has no year, so shows
/// sort after every dated movie.
pub async fn list_top_today(
    pool: &PgPool,
    limit: i64,
    access: &Restrictions,
) -> Result<Vec<MediaItem>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    let rows = sqlx::query_as!(
        CatalogRow,
        r#"
        SELECT c.id AS "id!", c.library_id AS "library_id!", c.item_type AS "item_type!",
               c.title AS "title!", c.year AS "year?", c.overview AS "overview?",
               c.season AS "season?", c.episode AS "episode?", c.created_at AS "created_at!"
        FROM (
            (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                    NULL::int AS season, NULL::int AS episode, m.created_at
             FROM media_items m
             WHERE m.item_type = 'movie' AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
             ORDER BY m.year DESC NULLS LAST, m.created_at DESC, m.id DESC
             LIMIT $1)
            UNION ALL
            (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int,
                    r.overview, NULL::int, NULL::int, s.last_episode_added_at
             FROM series s
             JOIN media_items r ON r.id = s.representative_item_id
             WHERE lisek_visible(s.library_id, r.age_rating, $2, $3, $4)
             ORDER BY s.last_episode_added_at DESC, s.representative_item_id DESC
             LIMIT $1)
        ) c
        ORDER BY c.year DESC NULLS LAST, c.created_at DESC, c.id DESC
        LIMIT $1
        "#,
        limit,
        libraries,
        max_age,
        block_unrated
    )
    .fetch_all(pool)
    .await?;

    Ok(into_items(rows))
}

/// "Recommended For You". No per-account taste model exists yet, so this
/// is an honest random sample. Each call draws a fresh sample.
pub async fn list_recommended(
    pool: &PgPool,
    library_id: Option<Uuid>,
    limit: i64,
    access: &Restrictions,
) -> Result<Vec<MediaItem>, DbError> {
    random_sample(pool, library_id, limit, access).await
}

/// "Popular Now" — same caveat as [`list_recommended`]: no view-count
/// tracking exists yet, so this is another independent random sample.
pub async fn list_popular(
    pool: &PgPool,
    library_id: Option<Uuid>,
    limit: i64,
    access: &Restrictions,
) -> Result<Vec<MediaItem>, DbError> {
    random_sample(pool, library_id, limit, access).await
}

/// Uniform random sample without `ORDER BY random()` over the whole table.
///
/// Every row carries a fixed random `shuffle_key` in [0, 1). A fresh seed
/// is drawn per call and the `limit` rows closest after it (circularly,
/// across movies and series together) are returned. Each branch reads at
/// most `limit` rows from an index: the rows at or after the seed, plus the
/// lowest keys to cover wrap-around. Neighbouring keys stay neighbours, so
/// two calls with nearby seeds overlap; for a "you might like" row that is
/// an acceptable trade for constant-time sampling.
async fn random_sample(
    pool: &PgPool,
    library_id: Option<Uuid>,
    limit: i64,
    access: &Restrictions,
) -> Result<Vec<MediaItem>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    let rows = match library_id {
        None => {
            sqlx::query_as!(
                CatalogRow,
                r#"
                WITH seed AS MATERIALIZED (SELECT random() AS v),
                candidates AS (
                    (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                            m.created_at, m.shuffle_key AS k
                     FROM media_items m
                     WHERE m.item_type = 'movie' AND m.shuffle_key >= (SELECT v FROM seed)
                       AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
                     ORDER BY m.shuffle_key LIMIT $1)
                    UNION ALL
                    (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                            m.created_at, m.shuffle_key
                     FROM media_items m
                     WHERE m.item_type = 'movie' AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
                     ORDER BY m.shuffle_key LIMIT $1)
                    UNION ALL
                    (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int,
                            r.overview, s.last_episode_added_at, s.shuffle_key
                     FROM series s
                     JOIN media_items r ON r.id = s.representative_item_id
                     WHERE s.shuffle_key >= (SELECT v FROM seed) AND lisek_visible(s.library_id, r.age_rating, $2, $3, $4)
                     ORDER BY s.shuffle_key LIMIT $1)
                    UNION ALL
                    (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int,
                            r.overview, s.last_episode_added_at, s.shuffle_key
                     FROM series s
                     JOIN media_items r ON r.id = s.representative_item_id
                     WHERE lisek_visible(s.library_id, r.age_rating, $2, $3, $4)
                     ORDER BY s.shuffle_key LIMIT $1)
                ),
                unique_candidates AS (
                    SELECT DISTINCT ON (id) * FROM candidates ORDER BY id
                )
                SELECT c.id AS "id!", c.library_id AS "library_id!", c.item_type AS "item_type!",
                       c.title AS "title!", c.year AS "year?", c.overview AS "overview?",
                       NULL::int AS "season?", NULL::int AS "episode?", c.created_at AS "created_at!"
                FROM unique_candidates c, seed
                ORDER BY (c.k - seed.v + 1.0) - floor(c.k - seed.v + 1.0)
                LIMIT $1
                "#,
                limit,
                libraries,
                max_age,
                block_unrated
            )
            .fetch_all(pool)
            .await?
        }
        Some(library_id) => {
            sqlx::query_as!(
                CatalogRow,
                r#"
                WITH seed AS MATERIALIZED (SELECT random() AS v),
                candidates AS (
                    (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                            m.created_at, m.shuffle_key AS k
                     FROM media_items m
                     WHERE m.item_type = 'movie' AND m.library_id = $2
                       AND m.shuffle_key >= (SELECT v FROM seed)
                       AND lisek_visible(m.library_id, m.age_rating, $3, $4, $5)
                     ORDER BY m.shuffle_key LIMIT $1)
                    UNION ALL
                    (SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                            m.created_at, m.shuffle_key
                     FROM media_items m
                     WHERE m.item_type = 'movie' AND m.library_id = $2
                       AND lisek_visible(m.library_id, m.age_rating, $3, $4, $5)
                     ORDER BY m.shuffle_key LIMIT $1)
                    UNION ALL
                    (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int,
                            r.overview, s.last_episode_added_at, s.shuffle_key
                     FROM series s
                     JOIN media_items r ON r.id = s.representative_item_id
                     WHERE s.library_id = $2 AND s.shuffle_key >= (SELECT v FROM seed)
                       AND lisek_visible(s.library_id, r.age_rating, $3, $4, $5)
                     ORDER BY s.shuffle_key LIMIT $1)
                    UNION ALL
                    (SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int,
                            r.overview, s.last_episode_added_at, s.shuffle_key
                     FROM series s
                     JOIN media_items r ON r.id = s.representative_item_id
                     WHERE s.library_id = $2 AND lisek_visible(s.library_id, r.age_rating, $3, $4, $5)
                     ORDER BY s.shuffle_key LIMIT $1)
                ),
                unique_candidates AS (
                    SELECT DISTINCT ON (id) * FROM candidates ORDER BY id
                )
                SELECT c.id AS "id!", c.library_id AS "library_id!", c.item_type AS "item_type!",
                       c.title AS "title!", c.year AS "year?", c.overview AS "overview?",
                       NULL::int AS "season?", NULL::int AS "episode?", c.created_at AS "created_at!"
                FROM unique_candidates c, seed
                ORDER BY (c.k - seed.v + 1.0) - floor(c.k - seed.v + 1.0)
                LIMIT $1
                "#,
                limit,
                library_id,
                libraries,
                max_age,
                block_unrated
            )
            .fetch_all(pool)
            .await?
        }
    };

    Ok(into_items(rows))
}

/// One item by id, in the public [`MediaItem`] shape — backs the
/// Movie/Series Detail View. An episode id resolves to its show's series
/// card (the same representative id the feed hands out), so an old link to
/// a single episode lands on the show's page. `None` when no such item
/// exists.
pub async fn get_media_item(
    pool: &PgPool,
    item_id: Uuid,
    access: &Restrictions,
) -> Result<Option<MediaItem>, DbError> {
    let (libraries, max_age, block_unrated) = access.binds();
    let row = sqlx::query_as!(
        CatalogRow,
        r#"
        SELECT c.id AS "id!", c.library_id AS "library_id!", c.item_type AS "item_type!",
               c.title AS "title!", c.year AS "year?", c.overview AS "overview?",
               c.season AS "season?", c.episode AS "episode?", c.created_at AS "created_at!"
        FROM (
            SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview,
                   m.season_number AS season, m.episode_number AS episode, m.created_at
            FROM media_items m
            WHERE m.id = $1 AND m.item_type <> 'episode'
              AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)
            UNION ALL
            SELECT s.representative_item_id, s.library_id, 'series', s.title, NULL::int, r.overview,
                   NULL::int, NULL::int, s.last_episode_added_at
            FROM episodes e
            JOIN series s ON s.id = e.series_id
            JOIN media_items r ON r.id = s.representative_item_id
            WHERE e.media_item_id = $1 AND lisek_visible(s.library_id, r.age_rating, $2, $3, $4)
        ) c
        LIMIT 1
        "#,
        item_id,
        libraries,
        max_age,
        block_unrated
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(MediaItem::from))
}

/// One season's episodes, as returned by [`list_series_seasons`].
pub struct SeriesSeason {
    pub season: i32,
    pub episodes: Vec<SeriesEpisode>,
}

pub struct SeriesEpisode {
    pub id: Uuid,
    pub episode: i32,
    /// Set only for a multi-episode file recognized as a range at scan
    /// time (e.g. `S01E01-E02`) — see `ScannedEpisode::end_episode`.
    pub end_episode: Option<i32>,
}

/// Every season (and, within each, every episode) of the show `item_id`
/// belongs to — any episode id of the show works, including the
/// representative one the feed hands out. Empty for a movie or an unknown
/// id. One call returns all seasons; the frontend expands them client-side.
///
/// Honest about what an episode "is" here: its id and numbers. No
/// per-episode title, still, runtime or synopsis exists yet — the scanner
/// only parses the series title, and no provider `/episodes` endpoint is
/// called — so the frontend labels each one generically ("Odcinek {n}").
pub async fn list_series_seasons(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Vec<SeriesSeason>, DbError> {
    let rows = sqlx::query!(
        r#"
        SELECT e.season_number, e.media_item_id, e.episode_number, e.end_episode_number
        FROM episodes e
        WHERE e.series_id = (SELECT series_id FROM episodes WHERE media_item_id = $1)
        ORDER BY e.season_number, e.episode_number, e.media_item_id
        "#,
        item_id
    )
    .fetch_all(pool)
    .await?;

    let mut seasons: Vec<SeriesSeason> = Vec::new();
    for row in rows {
        let episode = SeriesEpisode {
            id: row.media_item_id,
            episode: row.episode_number,
            end_episode: row.end_episode_number,
        };
        match seasons.last_mut() {
            Some(s) if s.season == row.season_number => s.episodes.push(episode),
            _ => seasons.push(SeriesSeason {
                season: row.season_number,
                episodes: vec![episode],
            }),
        }
    }
    Ok(seasons)
}

/// The "new" badge shown on a series' home-feed card — see
/// [`classify_badge_from_flags`]'s own doc comment for exactly what each
/// variant means and how it's decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesBadge {
    NewSeries,
    NewSeason,
    NewEpisodes,
}

impl SeriesBadge {
    pub fn as_str(self) -> &'static str {
        match self {
            SeriesBadge::NewSeries => "new_series",
            SeriesBadge::NewSeason => "new_season",
            SeriesBadge::NewEpisodes => "new_episodes",
        }
    }
}

/// A fixed 7-day "just added" window — arbitrary but documented. There's
/// no scan-batch id anywhere in this schema to say precisely which episodes
/// arrived together, so recency by `created_at` is the honest proxy for it.
/// `lisek-server::media::is_recently_added` reuses this window for a
/// movie's own "Nowość" badge, so both badge kinds agree.
pub const RECENT_BADGE_DAYS: i64 = 7;

/// Badges for many series cards in one query, keyed by the card id
/// (the series' representative item id). Cards without a badge are absent
/// from the map. Replaces one query per card.
pub async fn classify_series_badges(
    pool: &PgPool,
    card_ids: &[Uuid],
) -> Result<HashMap<Uuid, SeriesBadge>, DbError> {
    if card_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = sqlx::query!(
        r#"
        SELECT s.representative_item_id AS "card_id!",
               (e.created_at >= now() - make_interval(days => $2)) AS "is_recent!",
               e.season_number
        FROM series s
        JOIN episodes e ON e.series_id = s.id
        WHERE s.representative_item_id = ANY($1)
        "#,
        card_ids,
        RECENT_BADGE_DAYS as i32
    )
    .fetch_all(pool)
    .await?;

    let mut flags: HashMap<Uuid, Vec<(bool, Option<i32>)>> = HashMap::new();
    for row in rows {
        flags
            .entry(row.card_id)
            .or_default()
            .push((row.is_recent, Some(row.season_number)));
    }
    Ok(flags
        .into_iter()
        .filter_map(|(id, rows)| classify_badge_from_flags(&rows).map(|badge| (id, badge)))
        .collect())
}

/// Single-show variant of [`classify_series_badges`], by the show's
/// `(library_id, title)` key.
pub async fn classify_series_badge(
    pool: &PgPool,
    library_id: Uuid,
    title: &str,
) -> Result<Option<SeriesBadge>, DbError> {
    let card: Option<Option<Uuid>> = sqlx::query_scalar!(
        "SELECT representative_item_id FROM series WHERE library_id = $1 AND title = $2",
        library_id,
        title
    )
    .fetch_optional(pool)
    .await?;
    let Some(Some(card)) = card else {
        return Ok(None);
    };
    Ok(classify_series_badges(pool, &[card]).await?.remove(&card))
}

/// The pure classification logic behind [`classify_series_badges`], split
/// out so it's testable without a database — each tuple is one episode
/// row's `(is_recent, season)`.
///
///  - [`SeriesBadge::NewSeries`]: every episode of this show is within the
///    recent window — this show didn't exist in the library at all before
///    now, not even a season.
///  - [`SeriesBadge::NewSeason`]: some episodes are older than the window,
///    and at least one recent episode belongs to a season number no older
///    episode has — a season new to the library, not just new files inside
///    a season already there.
///  - [`SeriesBadge::NewEpisodes`]: recent episodes exist, but only within
///    season numbers already represented among the older ones.
///  - `None`: nothing for this show was added within the window.
fn classify_badge_from_flags(rows: &[(bool, Option<i32>)]) -> Option<SeriesBadge> {
    let mut old_seasons: HashSet<i32> = HashSet::new();
    let mut new_seasons: HashSet<i32> = HashSet::new();
    let mut has_old = false;
    let mut has_new = false;

    for &(is_recent, season) in rows {
        if is_recent {
            has_new = true;
            if let Some(s) = season {
                new_seasons.insert(s);
            }
        } else {
            has_old = true;
            if let Some(s) = season {
                old_seasons.insert(s);
            }
        }
    }

    if !has_new {
        return None;
    }
    if !has_old {
        return Some(SeriesBadge::NewSeries);
    }
    if new_seasons.difference(&old_seasons).next().is_some() {
        return Some(SeriesBadge::NewSeason);
    }
    Some(SeriesBadge::NewEpisodes)
}

/// Sort order for [`list_items_by_library_page`] — the small, fixed set of
/// ways the `/library/[id]` "all items" grid can be browsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibrarySort {
    TitleAsc,
    TitleDesc,
    RecentlyAdded,
}

/// Optional narrowing of a library page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LibraryFilter<'a> {
    /// Titles starting with this ASCII letter (case-insensitive). Backed by
    /// the generated `title_initial` column, so it is an index lookup.
    /// Titles starting with a non-ASCII letter match no letter button and
    /// stay reachable via the unfiltered view.
    pub letter: Option<char>,
    /// Case-insensitive substring of the title, served by the `pg_trgm`
    /// GIN indexes. `%`, `_` and `\` are matched literally.
    pub search: Option<&'a str>,
    /// Show the films of a collection (two or more visible ones in this
    /// library) as one `item_type = "collection"` card instead. Ignored
    /// while a letter or search filter is set: a search lists films.
    pub group_collections: bool,
}

impl LibraryFilter<'_> {
    fn grouping(&self) -> bool {
        self.group_collections && self.letter.is_none() && self.search.is_none()
    }
}

// Library pages combine three sorts with two optional filters — twelve
// literal variants as `query_as!` macros. Instead the SQL is assembled from
// the fixed fragments below; only placeholder numbers are formatted in and
// every value is bound, which is why `AssertSqlSafe` is sound here.
//
// Fixed placeholders: `$1` library, `$2..$4` the account's restrictions
// (see `access::Restrictions::binds`); filter values follow from `$5`.
const MOVIE_COLUMNS: &str = "SELECT m.id, m.library_id, m.item_type, m.title, m.year, m.overview, \
     NULL::int AS season, NULL::int AS episode, m.created_at \
     FROM media_items m WHERE m.item_type = 'movie' AND m.library_id = $1 \
     AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)";
const SERIES_COLUMNS: &str = "SELECT s.representative_item_id AS id, s.library_id, \
     'series'::text AS item_type, s.title, NULL::int AS year, r.overview, \
     NULL::int AS season, NULL::int AS episode, s.last_episode_added_at AS created_at \
     FROM series s JOIN media_items r ON r.id = s.representative_item_id WHERE s.library_id = $1 \
     AND lisek_visible(s.library_id, r.age_rating, $2, $3, $4)";
const FIRST_FILTER_PLACEHOLDER: usize = 5;

/// With collection grouping: `members` are the visible films of this
/// library that belong to a collection, `grouped` the collections with at
/// least two of them (one card each).
const GROUPED_CTE: &str = "WITH members AS ( \
       SELECT ci.collection_id, m.id, m.year, m.created_at \
       FROM collection_items ci JOIN media_items m ON m.id = ci.media_item_id \
       WHERE m.item_type = 'movie' AND m.library_id = $1 \
         AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4)), \
     grouped AS ( \
       SELECT collection_id, min(year) AS year, max(created_at) AS created_at \
       FROM members GROUP BY collection_id HAVING count(*) >= 2) ";
const NOT_GROUPED: &str = " AND NOT EXISTS (SELECT 1 FROM members g \
     JOIN grouped USING (collection_id) WHERE g.id = m.id)";
const COLLECTION_COLUMNS: &str = "SELECT k.id, $1::uuid AS library_id, 'collection'::text AS item_type, \
     k.name AS title, g.year, NULL::text AS overview, NULL::int AS season, NULL::int AS episode, \
     g.created_at FROM grouped g JOIN collections k ON k.id = g.collection_id";

/// `(movie branch, series branch, collection branch, merged)` ORDER BY
/// clauses — the same keys everywhere so the per-branch LIMIT is exact.
fn order_clauses(sort: LibrarySort) -> [&'static str; 4] {
    match sort {
        LibrarySort::TitleAsc => [
            "m.title ASC, m.id ASC",
            "s.title ASC, s.representative_item_id ASC",
            "k.name ASC, k.id ASC",
            "c.title ASC, c.id ASC",
        ],
        LibrarySort::TitleDesc => [
            "m.title DESC, m.id DESC",
            "s.title DESC, s.representative_item_id DESC",
            "k.name DESC, k.id DESC",
            "c.title DESC, c.id DESC",
        ],
        LibrarySort::RecentlyAdded => [
            "m.created_at DESC, m.id DESC",
            "s.last_episode_added_at DESC, s.representative_item_id DESC",
            "g.created_at DESC, k.id DESC",
            "c.created_at DESC, c.id DESC",
        ],
    }
}

/// Appends the filter conditions to both branches. Returns the extra SQL
/// for each branch plus the values to bind, in placeholder order starting
/// at [`FIRST_FILTER_PLACEHOLDER`].
fn filter_clauses(filter: &LibraryFilter<'_>) -> (String, String, Vec<String>) {
    let mut movie = String::new();
    let mut series = String::new();
    let mut binds = Vec::new();
    if let Some(letter) = filter.letter {
        binds.push(letter.to_ascii_uppercase().to_string());
        let n = binds.len() + FIRST_FILTER_PLACEHOLDER - 1;
        movie.push_str(&format!(" AND m.title_initial = ${n}"));
        series.push_str(&format!(" AND s.title_initial = ${n}"));
    }
    if let Some(search) = filter.search {
        binds.push(format!("%{}%", escape_like(search)));
        let n = binds.len() + FIRST_FILTER_PLACEHOLDER - 1;
        movie.push_str(&format!(" AND m.title ILIKE ${n}"));
        series.push_str(&format!(" AND s.title ILIKE ${n}"));
    }
    if filter.grouping() {
        movie.push_str(NOT_GROUPED);
    }
    (movie, series, binds)
}

pub(crate) fn escape_like(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// One page of a library's movies and series (and, with
/// `group_collections`, collections). Each branch fetches `offset + limit`
/// rows from its index, then the merged list is cut to the page — deep
/// pages cost more, the first pages stay constant.
pub async fn list_items_by_library_page(
    pool: &PgPool,
    library_id: Uuid,
    sort: LibrarySort,
    filter: LibraryFilter<'_>,
    limit: i64,
    offset: i64,
    access: &Restrictions,
) -> Result<Vec<MediaItem>, DbError> {
    let [movie_order, series_order, collection_order, merged_order] = order_clauses(sort);
    let (movie_filter, series_filter, binds) = filter_clauses(&filter);
    let window = binds.len() + FIRST_FILTER_PLACEHOLDER;
    let page_limit = window + 1;
    let page_offset = window + 2;
    let (cte, collection_branch) = if filter.grouping() {
        (
            GROUPED_CTE,
            format!(
                " UNION ALL ({COLLECTION_COLUMNS} ORDER BY {collection_order} LIMIT ${window})"
            ),
        )
    } else {
        ("", String::new())
    };
    let sql = format!(
        "{cte}SELECT c.id, c.library_id, c.item_type, c.title, c.year, c.overview, c.season, \
                c.episode, c.created_at \
         FROM (({MOVIE_COLUMNS}{movie_filter} ORDER BY {movie_order} LIMIT ${window}) \
               UNION ALL \
               ({SERIES_COLUMNS}{series_filter} ORDER BY {series_order} LIMIT ${window})\
               {collection_branch}) c \
         ORDER BY {merged_order} LIMIT ${page_limit} OFFSET ${page_offset}"
    );

    let (libraries, max_age, block_unrated) = access.binds();
    let mut query = sqlx::query_as::<_, CatalogRow>(AssertSqlSafe(sql))
        .bind(library_id)
        .bind(libraries)
        .bind(max_age)
        .bind(block_unrated);
    for value in binds {
        query = query.bind(value);
    }
    let rows = query
        .bind(offset.saturating_add(limit))
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    Ok(into_items(rows))
}

/// Total number of cards [`list_items_by_library_page`] pages through for
/// the same library and filter — a series and a grouped collection count
/// once.
pub async fn count_items_by_library(
    pool: &PgPool,
    library_id: Uuid,
    filter: LibraryFilter<'_>,
    access: &Restrictions,
) -> Result<i64, DbError> {
    let (movie_filter, series_filter, binds) = filter_clauses(&filter);
    let (cte, collections) = if filter.grouping() {
        (GROUPED_CTE, " + (SELECT count(*) FROM grouped)")
    } else {
        ("", "")
    };
    let sql = format!(
        "{cte}SELECT (SELECT count(*) FROM media_items m \
                 WHERE m.item_type = 'movie' AND m.library_id = $1 \
                 AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4){movie_filter}) \
              + (SELECT count(*) FROM series s JOIN media_items r ON r.id = s.representative_item_id \
                 WHERE s.library_id = $1 \
                 AND lisek_visible(s.library_id, r.age_rating, $2, $3, $4){series_filter}){collections}"
    );
    let (libraries, max_age, block_unrated) = access.binds();
    let mut query = sqlx::query_scalar::<_, i64>(AssertSqlSafe(sql))
        .bind(library_id)
        .bind(libraries)
        .bind(max_age)
        .bind(block_unrated);
    for value in binds {
        query = query.bind(value);
    }
    Ok(query.fetch_one(pool).await?)
}

/// Accented letters folded to their base capital for the alpha picker, so
/// "Łowcy" and "Émilie" are found under L and E. Pairs line up by position
/// (`ALPHA_FOLD_FROM[i]` → `ALPHA_FOLD_TO[i]`); a unit test checks the
/// lengths. Lower case is listed too: `upper()` leaves non-ASCII letters
/// alone under a `C` ctype.
const ALPHA_FOLD_FROM: &str =
    "ĄĆĘŁŃÓŚŹŻÀÁÂÃÄÅÇÈÉÊËÌÍÎÏÑÒÔÕÖØÙÚÛÜÝČĎĚŇŘŠŤŮŽąćęłńóśźżàáâãäåçèéêëìíîïñòôõöøùúûüýčďěňřšťůž";
const ALPHA_FOLD_TO: &str =
    "ACELNOSZZAAAAAACEEEEIIIINOOOOOUUUUYCDENRSTUZACELNOSZZAAAAAACEEEEIIIINOOOOOUUUUYCDENRSTUZ";

/// Where each letter starts in a library's "all items" list: the
/// position (0-based, in the given sort) of the first card whose title
/// begins with that letter. Letters are `A`..`Z` (accented capitals folded,
/// see [`ALPHA_FOLD_FROM`]) and `#` for everything else (digits, symbols,
/// other scripts). Only letters with at least one card are returned, in
/// list order.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct LetterIndex {
    pub letter: String,
    pub index: i64,
}

/// The alpha picker's jump table for [`list_items_by_library_page`]'s
/// unfiltered list: the same cards (series once, grouped collections as
/// one card), numbered with the same ORDER BY, so `index / page_size + 1`
/// is the page and `index % page_size` the card on it. One sort of the
/// library's titles; no per-letter queries.
pub async fn library_letter_index(
    pool: &PgPool,
    library_id: Uuid,
    sort: LibrarySort,
    group_collections: bool,
    access: &Restrictions,
) -> Result<Vec<LetterIndex>, DbError> {
    let [_, _, _, merged_order] = order_clauses(sort);
    let filter = LibraryFilter {
        group_collections,
        ..LibraryFilter::default()
    };
    let (movie_filter, series_filter, binds) = filter_clauses(&filter);
    debug_assert!(binds.is_empty());
    let (cte, collection_branch) = if filter.grouping() {
        (
            GROUPED_CTE,
            " UNION ALL SELECT k.id, k.name AS title, g.created_at \
             FROM grouped g JOIN collections k ON k.id = g.collection_id",
        )
    } else {
        ("", "")
    };
    let sql = format!(
        "{cte}SELECT letter, (min(rn) - 1)::bigint AS index FROM ( \
           SELECT CASE WHEN f.initial ~ '^[A-Z]$' THEN f.initial ELSE '#' END AS letter, \
                  row_number() OVER (ORDER BY {merged_order}) AS rn \
           FROM (SELECT m.id, m.title, m.created_at FROM media_items m \
                 WHERE m.item_type = 'movie' AND m.library_id = $1 \
                 AND lisek_visible(m.library_id, m.age_rating, $2, $3, $4){movie_filter} \
                 UNION ALL \
                 SELECT s.representative_item_id, s.title, s.last_episode_added_at \
                 FROM series s JOIN media_items r ON r.id = s.representative_item_id \
                 WHERE s.library_id = $1 \
                 AND lisek_visible(s.library_id, r.age_rating, $2, $3, $4){series_filter}\
                 {collection_branch}) c \
           CROSS JOIN LATERAL (SELECT translate(upper(left(c.title, 1)), $5, $6) AS initial) f \
         ) numbered GROUP BY letter ORDER BY 2"
    );
    let (libraries, max_age, block_unrated) = access.binds();
    Ok(sqlx::query_as::<_, LetterIndex>(AssertSqlSafe(sql))
        .bind(library_id)
        .bind(libraries)
        .bind(max_age)
        .bind(block_unrated)
        .bind(ALPHA_FOLD_FROM)
        .bind(ALPHA_FOLD_TO)
        .fetch_all(pool)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_fold_pairs_line_up() {
        assert_eq!(
            ALPHA_FOLD_FROM.chars().count(),
            ALPHA_FOLD_TO.chars().count()
        );
        assert!(ALPHA_FOLD_TO.chars().all(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn filters_number_their_placeholders_after_the_library_and_restrictions() {
        let (movie, series, binds) = filter_clauses(&LibraryFilter {
            letter: Some('m'),
            search: Some("50%_off\\"),
            group_collections: true,
        });
        assert_eq!(movie, " AND m.title_initial = $5 AND m.title ILIKE $6");
        assert_eq!(series, " AND s.title_initial = $5 AND s.title ILIKE $6");
        assert_eq!(binds, vec!["M".to_string(), "%50\\%\\_off\\\\%".to_string()]);
        assert_eq!(filter_clauses(&LibraryFilter::default()).2.len(), 0);
    }

    #[test]
    fn collection_grouping_applies_only_without_a_filter() {
        let grouped = LibraryFilter {
            group_collections: true,
            ..LibraryFilter::default()
        };
        assert!(grouped.grouping());
        assert_eq!(filter_clauses(&grouped).0, NOT_GROUPED);
        let searching = LibraryFilter {
            search: Some("star"),
            ..grouped
        };
        assert!(!searching.grouping());
        assert!(!filter_clauses(&searching).0.contains("NOT EXISTS"));
    }

    // `classify_badge_from_flags` is the pure decision logic behind
    // `classify_series_badges` — split out specifically so it's testable
    // without a live Postgres. `true`/`false` stands in for "this
    // episode's `created_at` falls inside the recent window or not"; the
    // `Option<i32>` is its season number.

    #[test]
    fn badge_is_new_series_when_every_episode_is_recent() {
        let rows = [(true, Some(1)), (true, Some(1)), (true, Some(2))];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewSeries)
        );
    }

    #[test]
    fn badge_is_none_when_nothing_is_recent() {
        let rows = [(false, Some(1)), (false, Some(1))];
        assert_eq!(classify_badge_from_flags(&rows), None);
    }

    #[test]
    fn badge_is_new_season_when_a_recent_episode_has_an_unseen_season_number() {
        // Season 1 already existed; season 2 just arrived.
        let rows = [(false, Some(1)), (false, Some(1)), (true, Some(2))];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewSeason)
        );
    }

    #[test]
    fn badge_is_new_episodes_when_recent_episodes_stay_within_a_known_season() {
        // Season 1 already existed and gained two more recent episodes.
        let rows = [(false, Some(1)), (true, Some(1)), (true, Some(1))];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewEpisodes)
        );
    }

    #[test]
    fn badge_ignores_a_missing_season_number_rather_than_panicking() {
        let rows = [(true, None)];
        assert_eq!(
            classify_badge_from_flags(&rows),
            Some(SeriesBadge::NewSeries)
        );
    }
}
