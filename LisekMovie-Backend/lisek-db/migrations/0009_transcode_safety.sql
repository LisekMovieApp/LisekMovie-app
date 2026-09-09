CREATE TABLE transcode_safety_config (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK(id),
    enabled BOOLEAN NOT NULL DEFAULT true,
    max_width INTEGER NOT NULL DEFAULT 1920 CHECK(max_width BETWEEN 1 AND 16384),
    max_height INTEGER NOT NULL DEFAULT 1080 CHECK(max_height BETWEEN 1 AND 16384),
    max_bitrate_kbps INTEGER CHECK(max_bitrate_kbps > 0)
);
INSERT INTO transcode_safety_config(id) VALUES(true);
CREATE TABLE transcode_kills (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES playback_sessions(id) ON DELETE CASCADE,
    item_id UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    source_width INTEGER,
    source_height INTEGER,
    source_bitrate_kbps INTEGER,
    killed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
