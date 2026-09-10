//! Login, tokens, API keys, permissions.
//!
//! Faza 2 added password hashing for the First-Run Wizard. Faza 3 adds
//! opaque session tokens for the `Authorization: LisekMovie Token=...`
//! header scheme (the project's chosen auth transport, portable across a
//! browser, a future mobile/TV app, or a headless API client) — generation
//! here, storage/lookup in `lisek-db`, and the actual HTTP extractor in
//! `lisek-server`, which is the only crate that knows about Axum.

use std::time::Duration;

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// A human-facing minimum password rule, deliberately simple — see the
/// architecture plan's rule against exposing regex-shaped requirements to
/// end users. The frontend enforces the same number so the requirement is
/// shown live, before the user ever submits the form.
pub const MIN_PASSWORD_LENGTH: usize = 8;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password is too long")]
    TooLong,
    #[error("password must be at least {MIN_PASSWORD_LENGTH} characters long")]
    TooShort,
    #[error("failed to hash password")]
    HashingFailed,
    #[error("stored password hash is invalid")]
    InvalidHash,
}

/// Hashes a plaintext password with Argon2id and a fresh random salt.
/// Returns the encoded hash (salt + parameters included) ready to store in
/// `users.password_hash`. The salt is generated internally, from the OS's
/// secure RNG, by `hash_password` itself — no salt handling on our side.
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    if password.len() > 1024 { return Err(PasswordError::TooLong); }
    if password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(PasswordError::TooShort);
    }

    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash: PasswordHash| hash.to_string())
        .map_err(|_| PasswordError::HashingFailed)
}

/// Verifies a plaintext password against a previously stored hash.
/// Returns `Ok(true)`/`Ok(false)` for a well-formed hash; `Err` only if the
/// stored hash itself is corrupt (a data problem, not a wrong-password one).
pub fn verify_password(password: &str, stored_hash: &str) -> Result<bool, PasswordError> {
    match Argon2::default().verify_password(password.as_bytes(), stored_hash) {
        Ok(()) => Ok(true),
        Err(password_hash::Error::PasswordInvalid) => Ok(false),
        Err(_) => Err(PasswordError::InvalidHash),
    }
}

/// How long a session stays valid after login, with no sliding renewal —
/// a fixed 30-day expiry is simple, safe enough for a self-hosted home
/// server, and avoids the extra moving parts of short-lived-access +
/// refresh-token rotation, which mainly earns its complexity in multi-service
/// OAuth ecosystems, not a single first-party web/API client. Logging back
/// in after expiry is a one-step, low-friction action here.
pub const SESSION_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 30);

/// Number of random bytes in a session token before encoding — 256 bits,
/// comfortably unguessable.
const SESSION_TOKEN_BYTES: usize = 32;

/// Generates a fresh, unguessable opaque session token (URL-safe base64,
/// unpadded — safe to put in a cookie or an `Authorization` header value
/// with no further escaping). This is the value handed to the client; only
/// [`hash_session_token`]'s output of it is ever stored.
pub fn generate_session_token() -> String {
    let mut bytes = [0u8; SESSION_TOKEN_BYTES];
    rand::fill(&mut bytes[..]);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Hashes a session token for storage/lookup — SHA-256 is fine here (unlike
/// password hashing, this isn't defending against offline brute-forcing of a
/// human-chosen secret: the input is already a full 256 bits of randomness,
/// so a fast hash loses nothing and keeps session lookups cheap). Never
/// store or log the raw token itself once this has been computed.
pub fn hash_session_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_scaffold_compiles() {
        assert_eq!(env!("CARGO_PKG_NAME"), "lisek-auth");
    }

    #[test]
    fn session_tokens_are_unique_and_url_safe() {
        let a = generate_session_token();
        let b = generate_session_token();
        assert_ne!(a, b);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn hashing_a_token_is_deterministic_but_one_way() {
        let token = generate_session_token();
        let hash_a = hash_session_token(&token);
        let hash_b = hash_session_token(&token);
        assert_eq!(hash_a, hash_b, "same token must hash the same way");
        assert_ne!(hash_a, token, "the hash must not just echo the token back");
    }

    #[test]
    fn hash_and_verify_round_trip() {
        let hash = hash_password("correct-horse-battery").unwrap();
        assert!(verify_password("correct-horse-battery", &hash).unwrap());
        assert!(!verify_password("wrong-password", &hash).unwrap());
    }

    #[test]
    fn rejects_passwords_shorter_than_the_minimum() {
        assert!(matches!(
            hash_password("short"),
            Err(PasswordError::TooShort)
        ));
    }

    #[test]
    fn two_hashes_of_the_same_password_differ() {
        // Different random salts each time — proves we're not doing anything
        // silly like a fixed/no salt.
        let a = hash_password("correct-horse-battery").unwrap();
        let b = hash_password("correct-horse-battery").unwrap();
        assert_ne!(a, b);
    }
}

/// 144 random bits, represented as a copyable 24-character temporary password.
pub fn generate_temporary_password() -> String {
    let mut bytes=[0u8;18];
    rand::fill(&mut bytes[..]);
    URL_SAFE_NO_PAD.encode(bytes)
}
