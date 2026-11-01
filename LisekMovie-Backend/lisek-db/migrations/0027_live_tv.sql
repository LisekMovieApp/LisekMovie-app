-- Faza 20: Live TV. A library of type `live_tv` has no folders: its
-- channels come from an M3U playlist and its programme guide (EPG) from an
-- XMLTV file, both refreshed by the server.

CREATE TABLE live_tv_sources (
    library_id     UUID PRIMARY KEY REFERENCES libraries(id) ON DELETE CASCADE,
    -- http(s) link or an absolute path on the server.
    m3u_url        TEXT NOT NULL,
    xmltv_url      TEXT,
    refreshed_at   TIMESTAMPTZ,
    last_error     TEXT,
    channel_count  INTEGER NOT NULL DEFAULT 0,
    program_count  INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE live_channels (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    library_id   UUID NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    -- Stable identity across refreshes: the tvg-id, else the stream URL.
    channel_key  TEXT NOT NULL,
    tvg_id       TEXT,
    name         TEXT NOT NULL,
    number       TEXT,
    group_title  TEXT,
    logo_url     TEXT,
    stream_url   TEXT NOT NULL,
    position     INTEGER NOT NULL,
    CONSTRAINT live_channels_key_unique UNIQUE (library_id, channel_key)
);
CREATE INDEX live_channels_library ON live_channels(library_id, position);

CREATE TABLE live_programs (
    id           BIGSERIAL PRIMARY KEY,
    channel_id   UUID NOT NULL REFERENCES live_channels(id) ON DELETE CASCADE,
    starts_at    TIMESTAMPTZ NOT NULL,
    ends_at      TIMESTAMPTZ NOT NULL CHECK (ends_at > starts_at),
    title        TEXT NOT NULL,
    subtitle     TEXT,
    description  TEXT,
    category     TEXT,
    icon_url     TEXT
);
CREATE INDEX live_programs_channel_time ON live_programs(channel_id, starts_at);
CREATE INDEX live_programs_window ON live_programs(ends_at, starts_at);
