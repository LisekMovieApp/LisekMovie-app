//! Tiny, dependency-free `.env` read/replace/write helper.
//!
//! `dotenvy` only reads `.env` files; we also need to write one, once, when
//! the First-Run Wizard finishes, and when a secret key is generated on
//! first start. Deliberately minimal: it only ever touches the one
//! `KEY=value` line it was asked to write and leaves everything else in
//! the file untouched, rather than pulling in a generic .env-writer crate.

use std::fs;
use std::io;
use std::path::Path;

const ENV_PATH: &str = ".env";

/// Writes (or updates) `DATABASE_URL=<url>` in `.env`, creating the file if
/// it doesn't exist yet. Any other lines already present are preserved.
pub fn save_database_url(url: &str) -> io::Result<()> {
    save_var("DATABASE_URL", url)
}

/// Writes (or updates) `KEY=value` in `.env`, keeping every other line.
pub fn save_var(key: &str, value: &str) -> io::Result<()> {
    save_var_at(Path::new(ENV_PATH), key, value)
}

/// Same as [`save_var`], but against an explicit path — kept separate so
/// tests never touch the process's current directory (and so never risk
/// racing each other or the real `.env`).
fn save_var_at(path: &Path, key: &str, value: &str) -> io::Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let prefix = format!("{key}=");

    let mut found = false;
    let mut lines: Vec<String> = existing
        .lines()
        .map(|line| {
            if line.starts_with(&prefix) {
                found = true;
                format!("{key}={value}")
            } else {
                line.to_string()
            }
        })
        .collect();

    if !found {
        lines.push(format!("{key}={value}"));
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
        save_var_at(&path, "DATABASE_URL", "postgres://a:b@localhost:5432/db").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "DATABASE_URL=postgres://a:b@localhost:5432/db\n");
        let _ = fs::remove_file(path);
    }

    #[test]
    fn updates_existing_key_in_place_and_keeps_other_lines() {
        let path = temp_env_path("update");
        fs::write(&path, "SOME_OTHER_KEY=1\nDATABASE_URL=old\nANOTHER=2\n").unwrap();
        save_var_at(&path, "DATABASE_URL", "postgres://new").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            "SOME_OTHER_KEY=1\nDATABASE_URL=postgres://new\nANOTHER=2\n"
        );
        let _ = fs::remove_file(path);
    }
}
