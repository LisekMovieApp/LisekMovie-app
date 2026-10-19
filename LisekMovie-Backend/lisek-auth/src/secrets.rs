//! Encryption of secrets the server must read back (metadata provider API
//! keys), as opposed to passwords and session tokens, which are only ever
//! hashed.
//!
//! XChaCha20-Poly1305 with a random 192-bit nonce per value: no nonce
//! bookkeeping, authenticated, and the associated data binds a ciphertext
//! to its owner (e.g. the provider key), so a sealed value copied to
//! another row fails to open. Stored form: `v1:` + base64url(nonce ‖
//! ciphertext ‖ tag).
//!
//! The 32-byte key comes from `LISEK_SECRET_KEY` (64 hex characters or
//! base64). Losing it does not lose data that matters — sealed values just
//! have to be entered again — but it must be backed up together with the
//! database to avoid that.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use thiserror::Error;

const PREFIX: &str = "v1:";
const NONCE_LEN: usize = 24;
pub const KEY_LEN: usize = 32;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("secret key must be 32 bytes, given as 64 hex characters or base64")]
    InvalidKey,
    #[error("value is not a sealed secret")]
    NotSealed,
    #[error("sealed secret is corrupt, or was sealed with a different key")]
    OpenFailed,
}

/// Seals and opens short secrets with one server-wide key.
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for SecretBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBox(..)")
    }
}

impl SecretBox {
    pub fn from_key(key: &[u8; KEY_LEN]) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new_from_slice(key).expect("key length is fixed"),
        }
    }

    /// Parses `LISEK_SECRET_KEY`'s format: 64 hex characters, or base64
    /// (standard or URL-safe) of exactly 32 bytes.
    pub fn from_encoded_key(encoded: &str) -> Result<Self, SecretError> {
        let encoded = encoded.trim();
        let bytes =
            if encoded.len() == 2 * KEY_LEN && encoded.bytes().all(|b| b.is_ascii_hexdigit()) {
                (0..KEY_LEN)
                    .map(|i| u8::from_str_radix(&encoded[2 * i..2 * i + 2], 16))
                    .collect::<Result<Vec<u8>, _>>()
                    .map_err(|_| SecretError::InvalidKey)?
            } else {
                STANDARD
                    .decode(encoded)
                    .or_else(|_| URL_SAFE_NO_PAD.decode(encoded.trim_end_matches('=')))
                    .map_err(|_| SecretError::InvalidKey)?
            };
        let key: [u8; KEY_LEN] = bytes.try_into().map_err(|_| SecretError::InvalidKey)?;
        Ok(Self::from_key(&key))
    }

    /// A fresh random key in the hex form `from_encoded_key` accepts.
    pub fn generate_encoded_key() -> String {
        let mut key = [0u8; KEY_LEN];
        rand::fill(&mut key[..]);
        key.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Encrypts `plaintext`; `context` must be passed again to [`open`].
    pub fn seal(&self, plaintext: &str, context: &str) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        rand::fill(&mut nonce[..]);
        let ciphertext = self
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: context.as_bytes(),
                },
            )
            .expect("encryption of an in-memory buffer cannot fail");
        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(sealed))
    }

    pub fn open(&self, sealed: &str, context: &str) -> Result<String, SecretError> {
        let encoded = sealed.strip_prefix(PREFIX).ok_or(SecretError::NotSealed)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| SecretError::OpenFailed)?;
        if bytes.len() <= NONCE_LEN {
            return Err(SecretError::OpenFailed);
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().map_err(|_| SecretError::OpenFailed)?;
        let plaintext = self
            .cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| SecretError::OpenFailed)?;
        String::from_utf8(plaintext).map_err(|_| SecretError::OpenFailed)
    }

    pub fn is_sealed(value: &str) -> bool {
        value.starts_with(PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret_box() -> SecretBox {
        SecretBox::from_encoded_key(&SecretBox::generate_encoded_key()).unwrap()
    }

    #[test]
    fn round_trips_and_never_stores_the_plaintext() {
        let secrets = secret_box();
        let sealed = secrets.seal("tmdb-key-123", "tmdb");
        assert!(SecretBox::is_sealed(&sealed));
        assert!(!sealed.contains("tmdb-key-123"));
        assert_eq!(secrets.open(&sealed, "tmdb").unwrap(), "tmdb-key-123");
    }

    #[test]
    fn same_plaintext_seals_differently_each_time() {
        let secrets = secret_box();
        assert_ne!(secrets.seal("x", "tmdb"), secrets.seal("x", "tmdb"));
    }

    #[test]
    fn rejects_wrong_context_wrong_key_and_tampering() {
        let secrets = secret_box();
        let sealed = secrets.seal("secret", "tmdb");
        assert_eq!(secrets.open(&sealed, "tvdb"), Err(SecretError::OpenFailed));
        assert_eq!(
            secret_box().open(&sealed, "tmdb"),
            Err(SecretError::OpenFailed)
        );
        let mut tampered = sealed.clone();
        tampered.pop();
        tampered.push(if sealed.ends_with('A') { 'B' } else { 'A' });
        assert_eq!(
            secrets.open(&tampered, "tmdb"),
            Err(SecretError::OpenFailed)
        );
        assert_eq!(secrets.open("plain", "tmdb"), Err(SecretError::NotSealed));
    }

    #[test]
    fn accepts_hex_and_base64_keys_only_of_the_right_length() {
        let hex = "00".repeat(32);
        assert!(SecretBox::from_encoded_key(&hex).is_ok());
        assert!(SecretBox::from_encoded_key(&STANDARD.encode([7u8; 32])).is_ok());
        assert_eq!(
            SecretBox::from_encoded_key("abc").unwrap_err(),
            SecretError::InvalidKey
        );
        assert_eq!(
            SecretBox::from_encoded_key(&STANDARD.encode([7u8; 16])).unwrap_err(),
            SecretError::InvalidKey
        );
    }
}
