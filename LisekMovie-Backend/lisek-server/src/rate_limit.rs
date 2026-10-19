//! In-memory limits on failed credential checks (login, password change).
//!
//! Fixed 15-minute windows per key. Keys are checked *before* the Argon2
//! verification, so a blocked client costs no hashing CPU. Counters live in
//! process memory: a restart clears them, which is acceptable for a home
//! server and keeps this free of extra infrastructure.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Failed logins from one address per window.
pub const MAX_FAILURES_PER_IP: u32 = 10;
/// Failed logins for one username per window, across all addresses. Higher
/// than the per-address limit so one attacker cannot lock a user out, low
/// enough to make distributed guessing impractical against Argon2.
pub const MAX_FAILURES_PER_USERNAME: u32 = 30;
/// Wrong "current password" attempts per account per window.
pub const MAX_PASSWORD_CHANGE_FAILURES: u32 = 10;
/// Above this many tracked keys, expired windows are pruned on insert.
const PRUNE_ABOVE: usize = 10_000;

#[derive(Default)]
pub struct FailureLimiter {
    windows: Mutex<HashMap<String, Window>>,
}

struct Window {
    started: Instant,
    failures: u32,
}

impl FailureLimiter {
    /// `Err(retry_after)` when any key is at its limit.
    pub fn check(&self, keys: &[(String, u32)]) -> Result<(), Duration> {
        let now = Instant::now();
        let windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        keys.iter()
            .filter_map(|(key, max)| {
                let window = windows.get(key)?;
                let elapsed = now.duration_since(window.started);
                (elapsed < WINDOW && window.failures >= *max).then(|| WINDOW - elapsed)
            })
            .max()
            .map_or(Ok(()), Err)
    }

    pub fn record_failure(&self, keys: &[(String, u32)]) {
        let now = Instant::now();
        let mut windows = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if windows.len() > PRUNE_ABOVE {
            windows.retain(|_, w| now.duration_since(w.started) < WINDOW);
        }
        for (key, _) in keys {
            let window = windows.entry(key.clone()).or_insert(Window {
                started: now,
                failures: 0,
            });
            if now.duration_since(window.started) >= WINDOW {
                *window = Window {
                    started: now,
                    failures: 0,
                };
            }
            window.failures += 1;
        }
    }
}

/// The two keys a login attempt is counted under.
pub fn login_keys(ip: IpAddr, username: &str) -> [(String, u32); 2] {
    [
        (format!("login-ip:{ip}"), MAX_FAILURES_PER_IP),
        (
            format!("login-user:{}", username.trim().to_lowercase()),
            MAX_FAILURES_PER_USERNAME,
        ),
    ]
}

pub fn password_change_key(user_id: uuid::Uuid) -> [(String, u32); 1] {
    [(
        format!("password-change:{user_id}"),
        MAX_PASSWORD_CHANGE_FAILURES,
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_after_the_limit_and_reports_retry_after() {
        let limiter = FailureLimiter::default();
        let keys = [("k".to_string(), 3)];
        for _ in 0..3 {
            assert!(limiter.check(&keys).is_ok());
            limiter.record_failure(&keys);
        }
        let retry = limiter.check(&keys).unwrap_err();
        assert!(retry <= WINDOW && retry > WINDOW - Duration::from_secs(5));
    }

    #[test]
    fn per_ip_and_per_username_limits_are_independent() {
        let limiter = FailureLimiter::default();
        let ip = "192.0.2.1".parse().unwrap();
        for _ in 0..MAX_FAILURES_PER_IP {
            limiter.record_failure(&login_keys(ip, "Alice"));
        }
        assert!(
            limiter.check(&login_keys(ip, "bob")).is_err(),
            "the address is blocked"
        );
        let other_ip = "192.0.2.2".parse().unwrap();
        assert!(
            limiter.check(&login_keys(other_ip, "alice")).is_ok(),
            "the username is not yet"
        );
    }

    #[test]
    fn usernames_are_case_insensitive() {
        let limiter = FailureLimiter::default();
        for i in 0..MAX_FAILURES_PER_USERNAME {
            let ip = IpAddr::from([10, 0, (i / 256) as u8, (i % 256) as u8]);
            limiter.record_failure(&login_keys(ip, " ADMIN "));
        }
        assert!(
            limiter
                .check(&login_keys("10.9.9.9".parse().unwrap(), "admin"))
                .is_err()
        );
    }
}
