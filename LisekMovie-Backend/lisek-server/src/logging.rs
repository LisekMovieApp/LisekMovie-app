//! Central logs (Faza 17): every `tracing` event goes to the console (as
//! before) and to one JSON-lines file a day under `<config>/logs/`
//! (`lisekmovie.YYYY-MM-DD.log`, a new file at midnight UTC). Files older
//! than [`RETENTION_DAYS`] are deleted.
//!
//! Sources, told apart by the event's target:
//!
//! - `ffmpeg` / `ffprobe`: the tools' stderr (`lisek_playback::tool_log`),
//!   messages start with `[FFMPEG]` / `[FFPROBE]`;
//! - `frontend`: errors reported by browsers (`POST /api/logs/client`),
//!   messages start with `[FRONTEND]`;
//! - everything else (lisek-server, the crates, sqlx): the backend.
//!
//! Administrators read the files in the web app (`GET /api/admin/logs`).
//!
//! The file level is `LISEK_LOG_LEVEL` (an `EnvFilter`, default `info`);
//! the console keeps `RUST_LOG`.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{ConnectInfo, Path as UrlPath, Query};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

use crate::auth::{AdminUser, AuthError, SessionUser};

pub const RETENTION_DAYS: u64 = 30;
const PREFIX: &str = "lisekmovie";
const SUFFIX: &str = "log";
/// At most this much of the end of one file is read for the viewer.
const MAX_READ_BYTES: u64 = 32 * 1024 * 1024;
const DEFAULT_LIMIT: usize = 2_000;
const MAX_LIMIT: usize = 20_000;

/// `<config>/logs`.
pub fn logs_dir() -> PathBuf {
    crate::state::config_dir().join("logs")
}

fn filter(variable: &str) -> EnvFilter {
    std::env::var(variable)
        .ok()
        .and_then(|value| EnvFilter::try_new(value).ok())
        .unwrap_or_else(|| EnvFilter::new("info"))
}

/// Console + daily files. The guard flushes the file writer and must live
/// as long as the process. Without a writable logs folder only the console
/// is used (with a warning there).
pub fn init() -> Option<WorkerGuard> {
    let console = tracing_subscriber::fmt::layer().with_filter(filter("RUST_LOG"));
    let dir = logs_dir();
    let appender = std::fs::create_dir_all(&dir).map_err(|err| err.to_string()).and_then(|()| {
        RollingFileAppender::builder()
            .rotation(Rotation::DAILY)
            .filename_prefix(PREFIX)
            .filename_suffix(SUFFIX)
            .max_log_files(RETENTION_DAYS as usize)
            .build(&dir)
            .map_err(|err| err.to_string())
    });
    match appender {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let file = tracing_subscriber::fmt::layer()
                .json()
                .with_current_span(false)
                .with_span_list(false)
                .with_ansi(false)
                .with_writer(writer)
                .with_filter(filter("LISEK_LOG_LEVEL"));
            tracing_subscriber::registry().with(console).with(file).init();
            tracing::info!(logs_dir = %dir.display(), retention_days = RETENTION_DAYS, "writing log files");
            Some(guard)
        }
        Err(err) => {
            tracing_subscriber::registry().with(console).init();
            tracing::warn!(logs_dir = %dir.display(), %err, "cannot write log files; console only");
            None
        }
    }
}

/// `lisekmovie.2026-10-27.log` -> `2026-10-27`.
fn date_of(file_name: &str) -> Option<&str> {
    let date = file_name.strip_prefix(&format!("{PREFIX}."))?.strip_suffix(&format!(".{SUFFIX}"))?;
    valid_date(date).then_some(date)
}

fn valid_date(date: &str) -> bool {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() && date.len() == 10
}

/// Deletes day files older than the retention (by the date in the name;
/// `max_log_files` alone only counts files).
pub fn prune(dir: &Path, today: chrono::NaiveDate) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let oldest = today - chrono::Days::new(RETENTION_DAYS - 1);
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(date) = name.to_str().and_then(date_of) else {
            continue;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            continue;
        };
        if date < oldest && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Daily retention sweep (also right after startup).
pub async fn run_retention() {
    loop {
        let dir = logs_dir();
        let today = chrono::Utc::now().date_naive();
        let removed = tokio::task::spawn_blocking(move || prune(&dir, today)).await.unwrap_or(0);
        if removed > 0 {
            tracing::info!(removed, "old log files deleted");
        }
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    }
}

// --- reading ---------------------------------------------------------------------------

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LogFile {
    /// `YYYY-MM-DD` (UTC).
    pub date: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LogFilesResponse {
    /// Newest first.
    pub files: Vec<LogFile>,
    pub retention_days: u64,
}

/// One log line, parsed.
#[derive(Debug, Clone, PartialEq, Serialize, utoipa::ToSchema)]
pub struct LogEntry {
    /// RFC 3339, UTC.
    pub timestamp: String,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`.
    pub level: String,
    /// `backend`, `frontend`, `ffmpeg`, `ffprobe`.
    pub source: String,
    pub target: String,
    pub message: String,
    /// The event's other fields (`item_id`, `err`, `url`...).
    #[schema(value_type = Object)]
    pub fields: Value,
}

pub fn source_of(target: &str) -> &'static str {
    match target {
        "ffmpeg" => "ffmpeg",
        "ffprobe" => "ffprobe",
        "frontend" => "frontend",
        _ => "backend",
    }
}

/// One JSON line of the file layer; lines that are not JSON (a crash
/// message, a hand-edited file) come back as backend INFO text.
pub fn parse_line(line: &str) -> Option<LogEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(line) else {
        return Some(LogEntry {
            timestamp: String::new(),
            level: "INFO".into(),
            source: "backend".into(),
            target: String::new(),
            message: line.to_string(),
            fields: Value::Object(Default::default()),
        });
    };
    let text = |object: &mut serde_json::Map<String, Value>, key: &str| match object.remove(key) {
        Some(Value::String(s)) => s,
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let timestamp = text(&mut object, "timestamp");
    let level = text(&mut object, "level").to_uppercase();
    let target = text(&mut object, "target");
    let mut fields = match object.remove("fields") {
        Some(Value::Object(fields)) => fields,
        _ => Default::default(),
    };
    let message = match fields.remove("message") {
        Some(Value::String(s)) => s,
        Some(other) => other.to_string(),
        None => String::new(),
    };
    Some(LogEntry {
        timestamp,
        level,
        source: source_of(&target).to_string(),
        target,
        message,
        fields: Value::Object(fields),
    })
}

fn split_list(value: Option<&str>) -> Option<Vec<String>> {
    let list: Vec<String> = value?
        .split(',')
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .collect();
    (!list.is_empty()).then_some(list)
}

/// The last `limit` entries of `text` that pass the filters, plus how many
/// passed in all.
pub fn read_entries(
    text: &str,
    sources: Option<&[String]>,
    levels: Option<&[String]>,
    limit: usize,
) -> (Vec<LogEntry>, usize) {
    let mut entries: Vec<LogEntry> = text
        .lines()
        .filter_map(parse_line)
        .filter(|e| sources.is_none_or(|s| s.contains(&e.source)))
        .filter(|e| levels.is_none_or(|l| l.iter().any(|x| x.eq_ignore_ascii_case(&e.level))))
        .collect();
    let total = entries.len();
    if entries.len() > limit {
        entries.drain(..entries.len() - limit);
    }
    (entries, total)
}

#[derive(Debug)]
pub struct LogsError(StatusCode, &'static str);

impl IntoResponse for LogsError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error_code": self.1 }))).into_response()
    }
}

/// The days with a log file (the last 30).
#[utoipa::path(get,path="/api/admin/logs",tag="admin",responses((status=200,body=LogFilesResponse),(status=401),(status=403)),security(("lisek_auth"=[])))]
pub async fn list(_admin: AdminUser) -> Json<LogFilesResponse> {
    let dir = logs_dir();
    let mut files: Vec<LogFile> = tokio::task::spawn_blocking(move || {
        std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        let date = date_of(&name)?.to_string();
                        let size_bytes = entry.metadata().ok()?.len();
                        Some(LogFile { date, size_bytes })
                    })
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    files.sort_by(|a, b| b.date.cmp(&a.date));
    files.truncate(RETENTION_DAYS as usize);
    Json(LogFilesResponse {
        files,
        retention_days: RETENTION_DAYS,
    })
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LogQuery {
    /// Comma-separated: `backend`, `frontend`, `ffmpeg`, `ffprobe`. All when absent.
    pub source: Option<String>,
    /// Comma-separated: `error`, `warn`, `info`, `debug`, `trace`. All when absent.
    pub level: Option<String>,
    /// The newest this many matching entries (default 2000, at most 20000).
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LogDayResponse {
    pub date: String,
    /// Oldest first (a console scrolls down to the newest).
    pub entries: Vec<LogEntry>,
    /// Matching entries in the part read; more than `entries` when cut.
    pub total: usize,
    /// Only the end of a very large file was read.
    pub partial: bool,
    pub size_bytes: u64,
}

async fn read_tail(path: &Path) -> std::io::Result<(String, bool, u64)> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();
    let partial = size > MAX_READ_BYTES;
    if partial {
        file.seek(std::io::SeekFrom::Start(size - MAX_READ_BYTES)).await?;
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).await?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if partial && let Some(first_newline) = text.find('\n') {
        // The cut fell inside a line.
        text.drain(..=first_newline);
    }
    Ok((text, partial, size))
}

/// One day's entries, filtered.
#[utoipa::path(get,path="/api/admin/logs/{date}",tag="admin",params(("date"=String,Path),LogQuery),responses((status=200,body=LogDayResponse),(status=400),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn day(
    _admin: AdminUser,
    UrlPath(date): UrlPath<String>,
    Query(query): Query<LogQuery>,
) -> Result<Json<LogDayResponse>, LogsError> {
    if !valid_date(&date) {
        return Err(LogsError(StatusCode::BAD_REQUEST, "invalid_date"));
    }
    let path = logs_dir().join(format!("{PREFIX}.{date}.{SUFFIX}"));
    let (text, partial, size_bytes) = read_tail(&path)
        .await
        .map_err(|_| LogsError(StatusCode::NOT_FOUND, "log_not_found"))?;
    let sources = split_list(query.source.as_deref());
    let levels = split_list(query.level.as_deref());
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let (entries, total) =
        tokio::task::spawn_blocking(move || read_entries(&text, sources.as_deref(), levels.as_deref(), limit))
            .await
            .map_err(|_| LogsError(StatusCode::INTERNAL_SERVER_ERROR, "read_failed"))?;
    Ok(Json(LogDayResponse {
        date,
        entries,
        total,
        partial,
        size_bytes,
    }))
}

/// The raw file, for download.
#[utoipa::path(get,path="/api/admin/logs/{date}/raw",tag="admin",params(("date"=String,Path)),responses((status=200,description="JSON lines"),(status=400),(status=401),(status=403),(status=404)),security(("lisek_auth"=[])))]
pub async fn raw(_admin: AdminUser, UrlPath(date): UrlPath<String>) -> Result<Response, LogsError> {
    if !valid_date(&date) {
        return Err(LogsError(StatusCode::BAD_REQUEST, "invalid_date"));
    }
    let name = format!("{PREFIX}.{date}.{SUFFIX}");
    let bytes = tokio::fs::read(logs_dir().join(&name))
        .await
        .map_err(|_| LogsError(StatusCode::NOT_FOUND, "log_not_found"))?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            (header::CACHE_CONTROL, "private, no-store".to_string()),
        ],
        bytes,
    )
        .into_response())
}

// --- browser errors ------------------------------------------------------------------------

/// Reports per client address and minute.
const CLIENT_REPORTS_PER_MINUTE: u32 = 30;
const MAX_MESSAGE: usize = 2_000;
const MAX_DETAIL: usize = 8_000;

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ClientLogRequest {
    /// `error` (default), `warn` or `info`.
    pub level: Option<String>,
    pub message: String,
    /// `error`, `unhandledrejection`, `console`, `handle_error`...
    pub kind: Option<String>,
    pub stack: Option<String>,
    /// The page the error happened on.
    pub url: Option<String>,
    /// Script and line (`app.js:12:5`).
    pub source: Option<String>,
}

static CLIENT_WINDOWS: Mutex<Option<HashMap<IpAddr, (Instant, u32)>>> = Mutex::new(None);

/// `true` while `ip` is under its per-minute budget (and counts this one).
fn allow_client_report(ip: IpAddr) -> bool {
    let mut guard = CLIENT_WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    let windows = guard.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    if windows.len() > 10_000 {
        windows.retain(|_, (started, _)| now.duration_since(*started) < Duration::from_secs(60));
    }
    let window = windows.entry(ip).or_insert((now, 0));
    if now.duration_since(window.0) >= Duration::from_secs(60) {
        *window = (now, 0);
    }
    window.1 += 1;
    window.1 <= CLIENT_REPORTS_PER_MINUTE
}

fn clip(value: &str, max: usize) -> String {
    let value = value.trim();
    match value.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &value[..cut]),
        None => value.to_string(),
    }
}

/// The browser's address: the web app's proxy passes it on in
/// `X-Forwarded-For`; only trusted when the request comes from this
/// machine or a private network (the web app), never from outside.
fn client_ip(peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let peer_ip = peer.ip();
    let internal = match peer_ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback(),
    };
    if internal
        && let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse().ok())
    {
        return forwarded;
    }
    peer_ip
}

/// A browser reports an error ("[FRONTEND]" in the server log). Signed-in
/// or not (the login page can fail too); limited per address.
#[utoipa::path(post,path="/api/logs/client",tag="system",request_body=ClientLogRequest,responses((status=204),(status=400),(status=429)))]
pub async fn client(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    session: Result<SessionUser, AuthError>,
    Json(report): Json<ClientLogRequest>,
) -> StatusCode {
    let ip = client_ip(peer, &headers);
    if !allow_client_report(ip) {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    let message = clip(&report.message, MAX_MESSAGE);
    if message.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    let user = session.ok().map(|s| s.user.username);
    let kind = report.kind.as_deref().map(|k| clip(k, 40)).unwrap_or_else(|| "error".into());
    let url = report.url.as_deref().map(|u| clip(u, 500));
    let script = report.source.as_deref().map(|s| clip(s, 500));
    let stack = report.stack.as_deref().map(|s| clip(s, MAX_DETAIL));
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|v| clip(v, 300));
    let client = ip.to_string();
    macro_rules! report {
        ($level:expr) => {
            tracing::event!(
                target: "frontend",
                $level,
                kind,
                url,
                script,
                stack,
                user,
                client,
                user_agent = agent,
                "[FRONTEND] {message}"
            )
        };
    }
    match report.level.as_deref().unwrap_or("error") {
        "warn" | "warning" => report!(tracing::Level::WARN),
        "info" => report!(tracing::Level::INFO),
        _ => report!(tracing::Level::ERROR),
    }
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_parse_into_entries_with_their_source() {
        let line = r#"{"timestamp":"2026-10-27T12:00:00.000001Z","level":"WARN","fields":{"message":"[FFMPEG] Invalid data","session":"abc"},"target":"ffmpeg"}"#;
        let entry = parse_line(line).unwrap();
        assert_eq!(entry.level, "WARN");
        assert_eq!(entry.source, "ffmpeg");
        assert_eq!(entry.message, "[FFMPEG] Invalid data");
        assert_eq!(entry.fields["session"], "abc");
        let backend = parse_line(r#"{"timestamp":"t","level":"INFO","fields":{"message":"hi"},"target":"lisek_server::identify"}"#).unwrap();
        assert_eq!(backend.source, "backend");
        let text = parse_line("thread 'main' panicked").unwrap();
        assert_eq!((text.source.as_str(), text.message.as_str()), ("backend", "thread 'main' panicked"));
        assert!(parse_line("  ").is_none());
    }

    #[test]
    fn filters_keep_the_newest_entries() {
        let text = [
            r#"{"level":"INFO","fields":{"message":"a"},"target":"lisek_server"}"#,
            r#"{"level":"ERROR","fields":{"message":"b"},"target":"frontend"}"#,
            r#"{"level":"WARN","fields":{"message":"c"},"target":"ffprobe"}"#,
            r#"{"level":"ERROR","fields":{"message":"d"},"target":"lisek_db"}"#,
        ]
        .join("\n");
        let (all, total) = read_entries(&text, None, None, 10);
        assert_eq!((all.len(), total), (4, 4));
        let errors = split_list(Some("error")).unwrap();
        let (found, _) = read_entries(&text, None, Some(&errors), 10);
        assert_eq!(found.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), ["b", "d"]);
        let sources = split_list(Some("frontend, FFPROBE")).unwrap();
        let (found, _) = read_entries(&text, Some(&sources), None, 10);
        assert_eq!(found.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), ["b", "c"]);
        let (last, total) = read_entries(&text, None, None, 2);
        assert_eq!((last.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), total), (vec!["c", "d"], 4));
    }

    #[test]
    fn day_files_are_kept_for_thirty_days() {
        assert_eq!(date_of("lisekmovie.2026-10-27.log"), Some("2026-10-27"));
        assert_eq!(date_of("lisekmovie.2026-13-01.log"), None);
        assert_eq!(date_of("other.2026-10-27.log"), None);
        let dir = std::env::temp_dir().join(format!("lisek-logs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["lisekmovie.2026-09-27.log", "lisekmovie.2026-09-28.log", "lisekmovie.2026-10-27.log", "notes.txt"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        assert_eq!(prune(&dir, today), 1);
        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["lisekmovie.2026-09-28.log", "lisekmovie.2026-10-27.log", "notes.txt"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn client_reports_are_limited_and_clipped() {
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        for _ in 0..CLIENT_REPORTS_PER_MINUTE {
            assert!(allow_client_report(ip));
        }
        assert!(!allow_client_report(ip));
        assert_eq!(clip("abcdef", 3), "abc…");
        let peer: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "198.51.100.7, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(peer, &headers), "198.51.100.7".parse::<IpAddr>().unwrap());
        let outside: SocketAddr = "203.0.113.1:5000".parse().unwrap();
        assert_eq!(client_ip(outside, &headers), "203.0.113.1".parse::<IpAddr>().unwrap());
    }
}
