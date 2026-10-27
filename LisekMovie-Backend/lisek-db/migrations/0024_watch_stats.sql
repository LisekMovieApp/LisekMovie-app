-- Watch statistics (Faza 13): every playback session records which
-- platform it came from (parsed from the User-Agent when it started), how
-- long the title is and whether it was watched to the end, so the admin
-- dashboard and the bot summary can tell finished from abandoned viewing
-- and where a viewer stopped (`position_seconds / duration_seconds`).
ALTER TABLE playback_sessions
    -- The raw header, capped at 512 characters; NULL for clients that sent none.
    ADD COLUMN user_agent TEXT CHECK (char_length(user_agent) <= 512),
    -- windows, macos, linux, android, ios, smart_tv, other, unknown.
    ADD COLUMN platform TEXT NOT NULL DEFAULT 'unknown',
    -- desktop, mobile, tablet, tv, unknown.
    ADD COLUMN device_type TEXT NOT NULL DEFAULT 'unknown',
    -- chrome, firefox, safari, edge, opera, samsung, or NULL.
    ADD COLUMN browser TEXT,
    -- Length of the title as the player reported it (0: not yet known).
    ADD COLUMN duration_seconds DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (duration_seconds >= 0),
    -- Watched to the end (the resume rules' "watched", or played to the end).
    ADD COLUMN completed BOOLEAN NOT NULL DEFAULT false;

-- Sessions from before this migration: length from the probed file, and
-- "completed" when the last position was within the final 10%.
UPDATE playback_sessions p
SET duration_seconds = m.duration_seconds,
    completed = p.position_seconds >= 0.9 * m.duration_seconds
FROM media_items m
WHERE m.id = p.item_id AND m.duration_seconds > 0;

-- Every statistics query is a time range first: this index drives them
-- (the existing per-user and per-item indexes serve those filters).
CREATE INDEX playback_sessions_started ON playback_sessions (started_at);

-- One read-only key for bots (Discord, Telegram) that fetch the summary
-- with `X-Lisek-Stats-Key`; only its SHA-256 is stored.
CREATE TABLE stats_api_key (
    id         BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    key_hash   TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
