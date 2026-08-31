//! Tiny, dependency-free `.env` read/replace/write helper.
//!
//! `dotenvy` only reads `.env` files; we also need to write one, once, when
//! the First-Run Wizard finishes. Deliberately minimal: it only ever
//! touches the single `DATABASE_URL` line and leaves everything else in the
//! file untouched, rather than pulling in a generic .env-writer crate for
//! one line.

use std::fs;
use std::io;
use std::path::Path;

const ENV_PATH: &str = ".env";
const KEY: &str = "DATABASE_URL";

/// Writes (or updates) `DATABASE_URL=<url>` in `.env`, creating the file if
/// it doesn't exist yet. Any other lines already present are preserved.
pub fn save_database_url(url: &str) -> io::Result<()> {
    save_database_url_at(Path::new(ENV_PATH), url)
}

/// Same as [`save_database_url`], but against an explicit path — kept
/// separate so tests never touch the process's current directory (and so
/// never risk racing each other or the real `.env`).
fn save_database_url_at(path: &Path, url: &str) -> io::Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let prefix = format!("{KEY}=");

    let mut found = false;
    let mut lines: Vec<String> = existing
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                found = true;
                format!("{KEY}={url}")
            } else {
                line.to_string()
            }
        })
        .collect();

    if !found {
        lines.push(format!("{KEY}={url}"));
    }

    fs::write(path, lines.join("\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A path in the OS temp dir, unique per test run — avoids touching the
    /// real project `.env` and avoids two tests colliding on one file.
    fn temp_env_path(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("lisek-env-test-{label}-{nanos}"))
    }

    #[test]
    fn creates_env_file_when_missing() {
        let path = temp_env_path("create");
        save_database_url_at(&path, "postgres://a:b@localhost:5432/db").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "DATABASE_URL=postgres://a:b@localhost:5432/db\n");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn updates_existing_key_in_place_and_keeps_other_lines() {
        let path = temp_env_path("update");
        fs::write(&path, "SOME_OTHER_KEY=1\nDATABASE_URL=old\nANOTHER=2\n").unwrap();
        save_database_url_at(&path, "postgres://new").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "SOME_OTHER_KEY=1\nDATABASE_URL=postgres://new\nANOTHER=2\n"
        );
        let _ = fs::remove_file(path);
    }
}
