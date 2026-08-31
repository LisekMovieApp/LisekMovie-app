//! Domain types and business logic: libraries, media items, users.
//!
//! Faza 2 adds `User` — enough to support the First-Run Wizard creating the
//! first admin account. The rest (libraries, media items) lands in later
//! phases per the architecture plan.

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

/// A local account. Never carries the password hash — that stays inside the
/// db/auth layers and must never be serialized back to a client.
///
/// Deliberately has no sqlx dependency or derive: lisek-db maps database rows
/// onto this struct field by field, so this crate stays free of any notion
/// of Postgres or SQL (see the architecture plan's crate boundaries).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub is_admin: bool,
    pub ui_language: String,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-core");
    }

    #[test]
    fn user_serializes_without_a_password_field() {
        let user = User {
            id: Uuid::nil(),
            username: "admin".to_string(),
            is_admin: true,
            ui_language: "en".to_string(),
            email: None,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&user).unwrap();
        assert!(!json.contains("password"));
    }
}
