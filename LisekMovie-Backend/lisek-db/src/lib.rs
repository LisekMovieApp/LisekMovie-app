//! PostgreSQL access layer (sqlx) and migrations.
//!
//! Scaffolded in Faza 1 (workspace init). Real implementation lands in Faza 2
//! per the LisekMovie architecture plan.

#[cfg(test)]
mod tests {
    /// Proves the crate compiles and its test harness runs — nothing more.
    /// Replace with real coverage once lisek-db grows actual logic.
    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-db");
    }
}
