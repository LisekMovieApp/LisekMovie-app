//! Watch statistics endpoints (Faza 13).
//!
//! - `GET /api/admin/stats/dashboard`: everything the admin dashboard
//!   shows for a date range, filterable by user, platform, device type,
//!   content type and title.
//! - `GET /api/admin/stats/sessions`: more rows of the dashboard's session
//!   list (where each viewer stopped).
//! - `GET /api/stats/summary?period=daily|weekly`: a light summary for
//!   Discord/Telegram bots — rankings with ready-made "1h 38m" strings, or
//!   with `format=discord` a webhook payload (two embeds) that can be
//!   posted as it is. Readable by an admin session or by the read-only
//!   `X-Lisek-Stats-Key` an admin creates (`/api/admin/stats/key`).
//!
//! Queries live in `lisek_db::stats`.

use axum::Json;
use axum::extract::{FromRequestParts, Query, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use chrono::{DateTime, TimeDelta, Utc};
use lisek_db::stats::{self as db, Content, StatsFilter};
use lisek_telemetry::client::{DeviceType, Platform};
use lisek_telemetry::duration::human;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::auth::{AdminUser, AuthError};
use crate::playback::{PlaybackError, pool};
use crate::state::SharedState;

/// Header carrying the bots' read-only key.
pub const STATS_KEY_HEADER: &str = "x-lisek-stats-key";
/// Longest range one request may cover.
const MAX_RANGE_DAYS: i64 = 1100;

fn bad(code: &'static str) -> PlaybackError {
    PlaybackError(StatusCode::BAD_REQUEST, code)
}

/// An admin session, or the bots' key in [`STATS_KEY_HEADER`].
pub struct StatsReader;

impl FromRequestParts<SharedState> for StatsReader {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &SharedState,
    ) -> Result<Self, Self::Rejection> {
        let key = parts
            .headers
            .get(STATS_KEY_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        let Some(key) = key else {
            return AdminUser::from_request_parts(parts, state)
                .await
                .map(|_| StatsReader);
        };
        let pool = state.pool().ok_or(AuthError::ServerNotReady)?;
        let stored = db::api_key(&pool).await.map_err(|_| AuthError::Internal)?;
        let presented = lisek_auth::hash_session_token(&key);
        match stored {
            Some((hash, _)) if constant_time_eq(hash.as_bytes(), presented.as_bytes()) => {
                Ok(StatsReader)
            }
            _ => Err(AuthError::InvalidToken),
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// --- Admin dashboard -------------------------------------------------------

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DashboardQuery {
    /// Start of the range (RFC 3339). Default: 30 days before `to`.
    pub from: Option<DateTime<Utc>>,
    /// End of the range, exclusive (RFC 3339). Default: now.
    pub to: Option<DateTime<Utc>>,
    pub user_id: Option<Uuid>,
    /// windows, macos, linux, android, ios, smart_tv, other, unknown.
    pub platform: Option<String>,
    /// desktop, mobile, tablet, tv, unknown.
    pub device_type: Option<String>,
    /// `movie` or `series`.
    pub content: Option<String>,
    /// A film, episode or series id.
    pub item: Option<Uuid>,
    /// IANA time zone for days and hours (default UTC).
    pub tz: Option<String>,
    /// Rows per ranking, 1–50 (default 10).
    pub limit: Option<i64>,
    /// Session list paging (sessions endpoint).
    pub offset: Option<i64>,
}

struct Parsed {
    filter: StatsFilter,
    timezone: String,
    limit: i64,
}

async fn parse(pool: &lisek_db::PgPool, q: &DashboardQuery) -> Result<Parsed, PlaybackError> {
    let to = q.to.unwrap_or_else(Utc::now);
    let from = q.from.unwrap_or(to - TimeDelta::days(30));
    if from >= to || to - from > TimeDelta::days(MAX_RANGE_DAYS) {
        return Err(bad("invalid_range"));
    }
    let platform = match q.platform.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(
            Platform::parse(v)
                .ok_or(bad("invalid_platform"))?
                .as_str()
                .to_owned(),
        ),
        None => None,
    };
    let device_type = match q.device_type.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(
            DeviceType::parse(v)
                .ok_or(bad("invalid_device_type"))?
                .as_str()
                .to_owned(),
        ),
        None => None,
    };
    let content = match q.content.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(Content::parse(v).ok_or(bad("invalid_content"))?),
        None => None,
    };
    let timezone =
        q.tz.clone()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "UTC".into());
    if !db::valid_timezone(pool, &timezone).await {
        return Err(bad("invalid_timezone"));
    }
    Ok(Parsed {
        filter: StatsFilter {
            from,
            to,
            user_id: q.user_id,
            platform,
            device_type,
            content,
            item: q.item,
        },
        timezone,
        limit: q.limit.unwrap_or(10).clamp(1, 50),
    })
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Dashboard {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub timezone: String,
    pub totals: db::Totals,
    pub days: Vec<db::DayStats>,
    pub hours: Vec<db::HourStats>,
    pub platforms: Vec<db::Slice>,
    pub device_types: Vec<db::Slice>,
    pub content: Vec<db::Slice>,
    pub users: Vec<db::UserRank>,
    pub top_movies: Vec<db::TitleRank>,
    pub top_series: Vec<db::TitleRank>,
    /// Abandoned sessions per tenth of the title (index 0: 0–10 %).
    pub dropoff: Vec<i64>,
    /// Newest first; more via `/api/admin/stats/sessions`.
    pub sessions: Vec<db::SessionRow>,
}

/// Rows of the dashboard's session list per request.
const SESSIONS_PAGE: i64 = 50;

#[utoipa::path(get, path = "/api/admin/stats/dashboard", params(DashboardQuery), responses((status = 200, body = Dashboard), (status = 400, description = "invalid_range / invalid_platform / invalid_device_type / invalid_content / invalid_timezone"), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn dashboard(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<Dashboard>, PlaybackError> {
    let pool = pool(&state).await?;
    let Parsed {
        filter: f,
        timezone,
        limit,
    } = parse(&pool, &query).await?;
    let (
        totals,
        days,
        hours,
        platforms,
        device_types,
        content,
        users,
        top_movies,
        top_series,
        dropoff,
        sessions,
    ) = tokio::try_join!(
        db::totals(&pool, &f),
        db::by_day(&pool, &f, &timezone),
        db::by_hour(&pool, &f, &timezone),
        db::by_platform(&pool, &f),
        db::by_device_type(&pool, &f),
        db::by_content(&pool, &f),
        db::top_users(&pool, &f, limit),
        db::top_movies(&pool, &f, limit),
        db::top_series(&pool, &f, limit),
        db::dropoff(&pool, &f),
        db::sessions(&pool, &f, SESSIONS_PAGE, 0),
    )?;
    Ok(Json(Dashboard {
        from: f.from,
        to: f.to,
        timezone,
        totals,
        days,
        hours,
        platforms,
        device_types,
        content,
        users,
        top_movies,
        top_series,
        dropoff,
        sessions,
    }))
}

#[utoipa::path(get, path = "/api/admin/stats/sessions", params(DashboardQuery), responses((status = 200, body = Vec<db::SessionRow>), (status = 400), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn sessions(
    _admin: AdminUser,
    State(state): State<SharedState>,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<Vec<db::SessionRow>>, PlaybackError> {
    let pool = pool(&state).await?;
    let parsed = parse(&pool, &query).await?;
    let offset = query.offset.unwrap_or(0).clamp(0, 1_000_000);
    Ok(Json(
        db::sessions(&pool, &parsed.filter, SESSIONS_PAGE, offset).await?,
    ))
}

// --- Bots' key ---------------------------------------------------------------

#[derive(Serialize, utoipa::ToSchema)]
pub struct StatsKeyStatus {
    pub configured: bool,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct NewStatsKey {
    /// Shown once; only its hash is stored. Send it as `X-Lisek-Stats-Key`.
    pub key: String,
    pub created_at: DateTime<Utc>,
}

#[utoipa::path(get, path = "/api/admin/stats/key", responses((status = 200, body = StatsKeyStatus), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn key_status(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<StatsKeyStatus>, PlaybackError> {
    let stored = db::api_key(&pool(&state).await?).await?;
    Ok(Json(StatsKeyStatus {
        configured: stored.is_some(),
        created_at: stored.map(|(_, at)| at),
    }))
}

/// Creates the bots' key, replacing (and so revoking) any previous one.
#[utoipa::path(post, path = "/api/admin/stats/key", responses((status = 200, body = NewStatsKey), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn create_key(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<Json<NewStatsKey>, PlaybackError> {
    let key = lisek_auth::generate_session_token();
    let created_at =
        db::set_api_key(&pool(&state).await?, &lisek_auth::hash_session_token(&key)).await?;
    Ok(Json(NewStatsKey { key, created_at }))
}

#[utoipa::path(delete, path = "/api/admin/stats/key", responses((status = 204), (status = 401), (status = 403)), security(("lisek_auth" = [])))]
pub async fn delete_key(
    _admin: AdminUser,
    State(state): State<SharedState>,
) -> Result<StatusCode, PlaybackError> {
    db::delete_api_key(&pool(&state).await?).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Bot summary ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Period {
    /// The 24 hours before `until`.
    Daily,
    /// The 7 days before `until`.
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    Pl,
    En,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SummaryQuery {
    /// `daily` (default) or `weekly`.
    #[param(value_type = Option<String>)]
    pub period: Option<Period>,
    /// End of the period (RFC 3339), default now. A bot run at midnight
    /// gets exactly the previous day or week.
    pub until: Option<DateTime<Utc>>,
    /// Entries per ranking, 1–25 (default 5).
    pub limit: Option<i64>,
    /// IANA time zone for the dates in labels (default Europe/Warsaw).
    pub tz: Option<String>,
    /// `json` (default) or `discord` (a webhook payload with embeds).
    pub format: Option<String>,
    /// Language of the Discord texts: `pl` (default) or `en`.
    #[param(value_type = Option<String>)]
    pub lang: Option<Lang>,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SummaryTotals {
    pub watched_seconds: f64,
    /// e.g. "8m 48s".
    pub watched: String,
    pub plays: i64,
    pub active_users: i64,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SummaryUser {
    pub rank: usize,
    pub username: String,
    pub watched_seconds: f64,
    pub watched: String,
    pub plays: i64,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SummaryContent {
    /// `movie` or `series`.
    pub content: String,
    pub watched_seconds: f64,
    pub watched: String,
    pub plays: i64,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SummaryTitle {
    pub rank: usize,
    /// `/media/{id}` (a series' representative episode).
    pub id: Uuid,
    pub title: String,
    pub year: Option<i32>,
    pub watched_seconds: f64,
    /// e.g. "1h 38m".
    pub watched: String,
    pub plays: i64,
    /// Different accounts that watched it.
    pub unique_users: i64,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct StatsSummary {
    pub period: Period,
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    /// "14.09 - 20.09" for a week, "20.09" for a day, in `timezone`.
    pub label: String,
    pub timezone: String,
    pub generated_at: DateTime<Utc>,
    /// `dd.mm.yyyy hh:mm` of `to` in `timezone`.
    pub generated_label: String,
    pub totals: SummaryTotals,
    pub users: Vec<SummaryUser>,
    pub by_content: Vec<SummaryContent>,
    pub top_movies: Vec<SummaryTitle>,
    pub top_series: Vec<SummaryTitle>,
}

fn titles(rows: Vec<db::TitleRank>) -> Vec<SummaryTitle> {
    rows.into_iter()
        .enumerate()
        .map(|(i, r)| SummaryTitle {
            rank: i + 1,
            id: r.id,
            title: r.title,
            year: r.year,
            watched: human(r.watched_seconds),
            watched_seconds: r.watched_seconds,
            plays: r.plays,
            unique_users: r.users,
        })
        .collect()
}

#[utoipa::path(get, path = "/api/stats/summary", params(SummaryQuery), responses((status = 200, body = StatsSummary, description = "JSON summary, or with format=discord a Discord webhook payload"), (status = 400), (status = 401, description = "No admin session and no valid X-Lisek-Stats-Key")), security(("lisek_auth" = [])))]
pub async fn summary(
    _reader: StatsReader,
    State(state): State<SharedState>,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<Value>, PlaybackError> {
    let pool = pool(&state).await?;
    let period = query.period.unwrap_or(Period::Daily);
    let to = query.until.unwrap_or_else(Utc::now);
    let from = to
        - match period {
            Period::Daily => TimeDelta::days(1),
            Period::Weekly => TimeDelta::days(7),
        };
    let limit = query.limit.unwrap_or(5).clamp(1, 25);
    let timezone = query
        .tz
        .clone()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "Europe/Warsaw".into());
    if !db::valid_timezone(&pool, &timezone).await {
        return Err(bad("invalid_timezone"));
    }
    let discord = match query.format.as_deref() {
        None | Some("json") => false,
        Some("discord") => true,
        Some(_) => return Err(bad("invalid_format")),
    };
    let f = StatsFilter::range(from, to);
    // The last day of the week is the one before `to`'s midnight.
    let last_day = to - TimeDelta::seconds(1);
    let (totals, users, content, movies, series, first_label, last_label, generated_label) = tokio::try_join!(
        db::totals(&pool, &f),
        db::top_users(&pool, &f, limit),
        db::by_content(&pool, &f),
        db::top_movies(&pool, &f, limit),
        db::top_series(&pool, &f, limit),
        db::day_month(&pool, from, &timezone),
        db::day_month(&pool, last_day, &timezone),
        db::date_time(&pool, to, &timezone),
    )?;
    let label = match period {
        Period::Daily => last_label,
        Period::Weekly => format!("{first_label} - {last_label}"),
    };
    let summary = StatsSummary {
        period,
        from,
        to,
        label,
        timezone,
        generated_at: Utc::now(),
        generated_label,
        totals: SummaryTotals {
            watched_seconds: totals.watched_seconds,
            watched: human(totals.watched_seconds),
            plays: totals.plays,
            active_users: totals.users,
        },
        users: users
            .into_iter()
            .enumerate()
            .map(|(i, u)| SummaryUser {
                rank: i + 1,
                username: u.username,
                watched: human(u.watched_seconds),
                watched_seconds: u.watched_seconds,
                plays: u.plays,
            })
            .collect(),
        by_content: content
            .into_iter()
            .map(|c| SummaryContent {
                content: c.key,
                watched: human(c.watched_seconds),
                watched_seconds: c.watched_seconds,
                plays: c.plays,
            })
            .collect(),
        top_movies: titles(movies),
        top_series: titles(series),
    };
    if discord {
        return Ok(Json(discord_payload(
            &summary,
            query.lang.unwrap_or(Lang::Pl),
        )));
    }
    Ok(Json(serde_json::to_value(summary).map_err(|_| {
        PlaybackError(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
    })?))
}

// --- Discord texts ---------------------------------------------------------------

/// Polish noun forms: 1 / 2–4 (not 12–14) / the rest.
fn polish_plural(n: i64, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    let (last, tens) = (n % 10, n % 100);
    if n == 1 {
        one
    } else if (2..=4).contains(&last) && !(12..=14).contains(&tens) {
        few
    } else {
        many
    }
}

fn medal(rank: usize) -> String {
    match rank {
        1 => "🥇".into(),
        2 => "🥈".into(),
        3 => "🥉".into(),
        n => format!("{n}."),
    }
}

/// Markdown-safe and short: titles and names go into bold text, so
/// Discord's markdown characters are escaped (`STR_XI` stays `STR_XI`
/// on screen) and long titles are cut at 80 characters.
fn clean(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().filter(|c| !c.is_control()).take(80) {
        if matches!(c, '*' | '_' | '~' | '`' | '|' | '>' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    if text.chars().count() > 80 {
        out.push('…');
    }
    out
}

/// A Discord webhook body with two embeds: who watched how much (and
/// films vs series), and the most popular films and series.
pub fn discord_payload(s: &StatsSummary, lang: Lang) -> Value {
    let pl = lang == Lang::Pl;
    let (stats_title, popular_title) = match (s.period, pl) {
        (Period::Daily, true) => (
            "📊 Statystyki oglądania — ostatnia doba".to_owned(),
            "🔥 Popularne — ostatnia doba".to_owned(),
        ),
        (Period::Weekly, true) => (
            format!("📊 Statystyki oglądania — tydzień {}", s.label),
            format!("🔥 Popularne w tygodniu {}", s.label),
        ),
        (Period::Daily, false) => (
            "📊 Watch statistics — last 24 hours".to_owned(),
            "🔥 Popular — last 24 hours".to_owned(),
        ),
        (Period::Weekly, false) => (
            format!("📊 Watch statistics — week {}", s.label),
            format!("🔥 Popular this week {}", s.label),
        ),
    };

    let mut lines: Vec<String> = s
        .users
        .iter()
        .map(|u| {
            let plays = if pl {
                format!(
                    "{} {}",
                    u.plays,
                    polish_plural(u.plays, "odtworzenie", "odtworzenia", "odtworzeń")
                )
            } else {
                format!(
                    "{} {}",
                    u.plays,
                    if u.plays == 1 { "play" } else { "plays" }
                )
            };
            format!(
                "{} **{}** — {} ({plays})",
                medal(u.rank),
                clean(&u.username),
                u.watched
            )
        })
        .collect();
    if lines.is_empty() {
        lines.push(
            if pl {
                "Nikt nic nie oglądał."
            } else {
                "Nothing was watched."
            }
            .into(),
        );
    }
    let types: Vec<String> = s
        .by_content
        .iter()
        .map(|c| {
            let (icon, name) = match (c.content.as_str(), pl) {
                ("series", true) => ("📺", "Seriale"),
                ("series", false) => ("📺", "Series"),
                (_, true) => ("🎬", "Filmy"),
                (_, false) => ("🎬", "Films"),
            };
            format!("{icon} {name}: {} ({})", c.watched, c.plays)
        })
        .collect();
    if !types.is_empty() {
        lines.push(String::new());
        lines.push(
            if pl {
                "**Podział wg typu treści**"
            } else {
                "**By content type**"
            }
            .into(),
        );
        lines.extend(types);
    }
    let users_word = if pl {
        format!(
            "{} {}",
            s.totals.active_users,
            polish_plural(
                s.totals.active_users,
                "aktywny użytkownik",
                "aktywni użytkownicy",
                "aktywnych użytkowników"
            )
        )
    } else {
        format!(
            "{} active {}",
            s.totals.active_users,
            if s.totals.active_users == 1 {
                "user"
            } else {
                "users"
            }
        )
    };
    let total_word = if pl { "Łącznie" } else { "Total" };

    let ranking = |rows: &[SummaryTitle]| -> Vec<String> {
        rows.iter()
            .map(|t| {
                format!(
                    "{} **{}** — {} ({}x, {} 👤)",
                    medal(t.rank),
                    clean(&t.title),
                    t.watched,
                    t.plays,
                    t.unique_users
                )
            })
            .collect()
    };
    let mut popular: Vec<String> = Vec::new();
    for (icon, heading, rows) in [
        (
            "🎬",
            if pl {
                "Najpopularniejsze filmy"
            } else {
                "Most watched films"
            },
            &s.top_movies,
        ),
        (
            "📺",
            if pl {
                "Najpopularniejsze seriale"
            } else {
                "Most watched series"
            },
            &s.top_series,
        ),
    ] {
        if rows.is_empty() {
            continue;
        }
        if !popular.is_empty() {
            popular.push(String::new());
        }
        popular.push(format!("{icon} **{heading}**"));
        popular.extend(ranking(rows));
    }
    if popular.is_empty() {
        popular.push(
            if pl {
                "Brak odtworzeń w tym okresie."
            } else {
                "No plays in this period."
            }
            .into(),
        );
    }

    json!({
        "embeds": [
            {
                "title": stats_title,
                "description": lines.join("\n"),
                "color": 0x1ABC9C,
                "footer": { "text": format!("{total_word}: {} • {users_word} • {}", s.totals.watched, s.generated_label) },
            },
            {
                "title": popular_title,
                "description": popular.join("\n"),
                "color": 0x3498DB,
                "footer": { "text": s.generated_label },
            }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(period: Period) -> StatsSummary {
        let at = Utc::now();
        let title = |rank, name: &str, seconds: f64, plays, users| SummaryTitle {
            rank,
            id: Uuid::nil(),
            title: name.into(),
            year: None,
            watched_seconds: seconds,
            watched: human(seconds),
            plays,
            unique_users: users,
        };
        StatsSummary {
            period,
            from: at,
            to: at,
            label: "14.09 - 20.09".into(),
            timezone: "Europe/Warsaw".into(),
            generated_at: at,
            generated_label: "21.09.2026 00:00".into(),
            totals: SummaryTotals {
                watched_seconds: 528.0,
                watched: "8m 48s".into(),
                plays: 2,
                active_users: 1,
            },
            users: vec![SummaryUser {
                rank: 1,
                username: "STR_XI".into(),
                watched_seconds: 528.0,
                watched: "8m 48s".into(),
                plays: 2,
            }],
            by_content: vec![SummaryContent {
                content: "movie".into(),
                watched_seconds: 528.0,
                watched: "8m 48s".into(),
                plays: 2,
            }],
            top_movies: vec![
                title(1, "Koralina i tajemnice drzwi", 5880.0, 1, 1),
                title(2, "Transformers. Ostatni rycerz", 3270.0, 2, 1),
                title(3, "Transformers. Zemsta upadłych", 1704.0, 1, 1),
                title(4, "Kapitan Ameryka: Pierwsze starcie", 1619.0, 2, 1),
            ],
            top_series: vec![
                title(1, "Gra o tron", 11640.0, 10, 1),
                title(2, "Miasteczko *South* Park", 7740.0, 8, 2),
            ],
        }
    }

    #[test]
    fn discord_payload_reads_like_the_reference_report() {
        let payload = discord_payload(&sample(Period::Weekly), Lang::Pl);
        let stats = payload["embeds"][0]["description"].as_str().unwrap();
        assert!(
            stats.starts_with("🥇 **STR\\_XI** — 8m 48s (2 odtworzenia)"),
            "{stats}"
        );
        assert!(
            stats.contains("**Podział wg typu treści**\n🎬 Filmy: 8m 48s (2)"),
            "{stats}"
        );
        assert_eq!(
            payload["embeds"][0]["footer"]["text"],
            "Łącznie: 8m 48s • 1 aktywny użytkownik • 21.09.2026 00:00"
        );
        assert_eq!(
            payload["embeds"][1]["title"],
            "🔥 Popularne w tygodniu 14.09 - 20.09"
        );
        let popular = payload["embeds"][1]["description"].as_str().unwrap();
        assert!(popular.starts_with("🎬 **Najpopularniejsze filmy**\n🥇 **Koralina i tajemnice drzwi** — 1h 38m (1x, 1 👤)"), "{popular}");
        assert!(
            popular.contains("\n4. **Kapitan Ameryka: Pierwsze starcie** — 26m 59s (2x, 1 👤)"),
            "{popular}"
        );
        assert!(
            popular.contains(
                "\n\n📺 **Najpopularniejsze seriale**\n🥇 **Gra o tron** — 3h 14m (10x, 1 👤)"
            ),
            "{popular}"
        );
        assert!(
            popular.contains("**Miasteczko \\*South\\* Park**"),
            "markdown escaped: {popular}"
        );
        let en = discord_payload(&sample(Period::Daily), Lang::En);
        assert_eq!(
            en["embeds"][0]["title"],
            "📊 Watch statistics — last 24 hours"
        );
        assert!(
            en["embeds"][0]["description"]
                .as_str()
                .unwrap()
                .contains("(2 plays)")
        );
    }

    #[test]
    fn empty_periods_say_so() {
        let mut empty = sample(Period::Daily);
        empty.users.clear();
        empty.by_content.clear();
        empty.top_movies.clear();
        empty.top_series.clear();
        let payload = discord_payload(&empty, Lang::Pl);
        assert_eq!(payload["embeds"][0]["description"], "Nikt nic nie oglądał.");
        assert_eq!(
            payload["embeds"][1]["description"],
            "Brak odtworzeń w tym okresie."
        );
    }

    #[test]
    fn polish_plurals() {
        let f = |n| polish_plural(n, "odtworzenie", "odtworzenia", "odtworzeń");
        assert_eq!(
            [f(1), f(2), f(4), f(5), f(12), f(22), f(0)],
            [
                "odtworzenie",
                "odtworzenia",
                "odtworzenia",
                "odtworzeń",
                "odtworzeń",
                "odtworzenia",
                "odtworzeń"
            ]
        );
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}
