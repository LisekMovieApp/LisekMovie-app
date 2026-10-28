-- Faza 19: profile avatars, online status and the server's default language.

-- Set when the account uploads an avatar (milliseconds since the epoch,
-- also the cache-busting version in the avatar URL); NULL means no avatar.
-- The image itself lives in `config/avatars/<user id>.jpg`.
ALTER TABLE users ADD COLUMN avatar_version BIGINT;

-- Presence: `sessions.last_seen_at` (0002) is now refreshed by every
-- authenticated request (throttled to once a minute per session). The
-- admin user list reads the newest one per account.
CREATE INDEX idx_sessions_user_last_seen ON sessions(user_id, last_seen_at DESC);

-- One row: server-wide settings. `default_language` is the language the
-- First-Run Wizard was completed in; metadata providers fetch descriptions
-- and artwork in it, whatever language a single account uses for the UI.
CREATE TABLE server_settings (
    id               BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    default_language TEXT NOT NULL DEFAULT 'en' CHECK (default_language IN ('en', 'pl'))
);

-- Existing servers: the first administrator's language, which is the one
-- the wizard was run in.
INSERT INTO server_settings (id, default_language)
SELECT true, COALESCE(
    (SELECT ui_language FROM users WHERE is_admin AND ui_language IN ('en', 'pl') ORDER BY created_at, id LIMIT 1),
    'en'
);
