-- Login sessions, created by POST /api/auth/login and validated by every
-- protected endpoint. Token itself is never stored — only its SHA-256 hash,
-- the same "never persist the secret in recoverable form" principle already
-- applied to user passwords.

CREATE TABLE sessions (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash   TEXT NOT NULL UNIQUE,
    -- Free-text client identity, supplied by the caller at login time (a
    -- browser, a future mobile/TV app, a headless API client). Never
    -- validated against a fixed list — this is metadata for a future
    -- "your active sessions" screen, not an access-control input.
    client       TEXT NOT NULL DEFAULT 'unknown',
    device       TEXT NOT NULL DEFAULT 'unknown',
    device_id    TEXT NOT NULL DEFAULT 'unknown',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
);

-- Every auth check looks up a session by user first (via the FK) and by
-- token_hash (already unique-indexed via the column constraint) — this
-- index is for the "list my active sessions" / "log out this device"
-- screen planned for later, not for the per-request auth check itself.
CREATE INDEX idx_sessions_user_id ON sessions(user_id);
