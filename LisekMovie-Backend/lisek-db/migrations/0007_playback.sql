CREATE TABLE user_media_state (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    position_seconds DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (position_seconds >= 0),
    duration_seconds DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (duration_seconds >= 0),
    watched BOOLEAN NOT NULL DEFAULT false,
    favorite BOOLEAN NOT NULL DEFAULT false,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, item_id)
);

CREATE TABLE playback_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    auth_token_hash TEXT NOT NULL,
    ticket_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '6 hours',
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at TIMESTAMPTZ,
    paused BOOLEAN NOT NULL DEFAULT true,
    watched_seconds DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (watched_seconds >= 0),
    play_method TEXT NOT NULL DEFAULT 'direct_play',
    client_name TEXT NOT NULL DEFAULT 'unknown',
    device_name TEXT NOT NULL DEFAULT 'unknown',
    device_id TEXT NOT NULL DEFAULT 'unknown'
);
CREATE INDEX playback_sessions_user ON playback_sessions(user_id, started_at DESC);
CREATE INDEX playback_sessions_item ON playback_sessions(item_id, started_at DESC);
CREATE INDEX user_media_state_recent ON user_media_state(user_id, updated_at DESC);
