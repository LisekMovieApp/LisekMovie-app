-- Faza 26: own TV channels (an ErsatzTV-style scheduled channel built from
-- the libraries), ads/fillers from server folders, and the extensions the
-- Faza 20 IPTV channels need to join the same lineup.
--
-- A channel has one schedule (`tv_slots`). The schedule engine (`lisek-tv`)
-- turns it into a timeline, `tv_playout_items`, a few days ahead; "what
-- plays on channel X at time T" is one indexed lookup in that table.
-- Everything here is additive: no existing table loses a column or a row.

-- 1. Fillers (ads, bumpers) -------------------------------------------------
-- Folders are absolute paths on the server, like library folders. Files are
-- probed once; `duration_ms` is what the engine plans with.
CREATE TABLE tv_filler_folders (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    path        TEXT NOT NULL UNIQUE,
    enabled     BOOLEAN NOT NULL DEFAULT true,
    scanned_at  TIMESTAMPTZ,
    file_count  INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE tv_filler_files (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    folder_id    UUID NOT NULL REFERENCES tv_filler_folders(id) ON DELETE CASCADE,
    path         TEXT NOT NULL,
    duration_ms  INTEGER NOT NULL CHECK (duration_ms > 0),
    CONSTRAINT tv_filler_files_unique UNIQUE (folder_id, path)
);

-- A preset says how a set of folders is inserted: `count` files, about
-- `duration_seconds` of files, or `pad`: enough to reach the next full
-- `pad_minutes` (for example the full half hour). `folder_ids` has no
-- foreign key (an array); deleting a folder removes it from the arrays.
CREATE TABLE tv_filler_presets (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL UNIQUE CHECK (char_length(name) BETWEEN 1 AND 100),
    kind             TEXT NOT NULL CHECK (kind IN ('pre', 'mid', 'post', 'tail', 'fallback')),
    mode             TEXT NOT NULL CHECK (mode IN ('count', 'duration', 'pad')),
    count            INTEGER CHECK (count BETWEEN 1 AND 50),
    duration_seconds INTEGER CHECK (duration_seconds BETWEEN 1 AND 86400),
    pad_minutes      INTEGER CHECK (pad_minutes BETWEEN 1 AND 240),
    folder_ids       UUID[] NOT NULL DEFAULT '{}',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT tv_filler_presets_mode_fields CHECK (
        (mode = 'count' AND count IS NOT NULL) OR
        (mode = 'duration' AND duration_seconds IS NOT NULL) OR
        (mode = 'pad' AND pad_minutes IS NOT NULL))
);

-- 2. Channels ---------------------------------------------------------------
CREATE TABLE tv_channels (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    -- "5" or "5.1": what remote controls and IPTV players show.
    number             TEXT NOT NULL UNIQUE CHECK (number ~ '^[0-9]{1,5}(\.[0-9]{1,2})?$'),
    sort_number        NUMERIC(7, 2) GENERATED ALWAYS AS (number::numeric) STORED,
    name               TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    group_title        TEXT,
    -- A file in the config folder (`tv/logos/<id>`), set by the upload.
    logo_path          TEXT,
    enabled            BOOLEAN NOT NULL DEFAULT true,
    position           INTEGER NOT NULL DEFAULT 0,
    -- IANA zone; fixed start times in the schedule are local to it.
    timezone           TEXT NOT NULL DEFAULT 'UTC',
    quality            TEXT NOT NULL DEFAULT '720p' CHECK (quality IN ('480p', '720p', '1080p')),
    -- Plays instead of a gap or a file that cannot be read.
    fallback_filler_id UUID REFERENCES tv_filler_presets(id) ON DELETE SET NULL,
    -- Keep the stream running without viewers.
    keep_running       BOOLEAN NOT NULL DEFAULT false,
    -- Seed of every shuffle, so a rebuild gives the same timeline.
    seed               BIGINT NOT NULL DEFAULT (random() * 9.0e18)::bigint,
    -- Raised by every schedule edit; the builder compares it with
    -- `tv_channel_state.built_version`.
    schedule_version   INTEGER NOT NULL DEFAULT 1,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX tv_channels_order ON tv_channels (position, sort_number);

-- 3. Schedule slots ---------------------------------------------------------
CREATE TABLE tv_slots (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    channel_id       UUID NOT NULL REFERENCES tv_channels(id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    -- Local time of day (channel zone); NULL: starts right after the
    -- previous slot.
    start_time       TIME,
    start_mode       TEXT NOT NULL DEFAULT 'strict' CHECK (start_mode IN ('strict', 'flexible')),
    -- Monday = bit 0 ... Sunday = bit 6.
    days_of_week     SMALLINT NOT NULL DEFAULT 127 CHECK (days_of_week BETWEEN 1 AND 127),
    -- Not a foreign key: it points into libraries, series, media_items or
    -- collections. A source that is gone is skipped with a warning.
    source_kind      TEXT NOT NULL CHECK (source_kind IN ('library', 'series', 'movie', 'collection')),
    source_id        UUID NOT NULL,
    play_order       TEXT NOT NULL DEFAULT 'chronological'
        CHECK (play_order IN ('chronological', 'season_episode', 'shuffle', 'random')),
    mode             TEXT NOT NULL DEFAULT 'one' CHECK (mode IN ('one', 'multiple', 'duration', 'flood')),
    count            INTEGER CHECK (count BETWEEN 1 AND 100),
    duration_minutes INTEGER CHECK (duration_minutes BETWEEN 1 AND 1440),
    tail_mode        TEXT NOT NULL DEFAULT 'offline' CHECK (tail_mode IN ('offline', 'filler', 'overrun')),
    pre_filler_id    UUID REFERENCES tv_filler_presets(id) ON DELETE SET NULL,
    mid_filler_id    UUID REFERENCES tv_filler_presets(id) ON DELETE SET NULL,
    post_filler_id   UUID REFERENCES tv_filler_presets(id) ON DELETE SET NULL,
    tail_filler_id   UUID REFERENCES tv_filler_presets(id) ON DELETE SET NULL,
    title_override   TEXT,
    in_guide         BOOLEAN NOT NULL DEFAULT true,
    CONSTRAINT tv_slots_mode_fields CHECK (
        (mode = 'multiple' AND count IS NOT NULL) OR
        (mode = 'duration' AND duration_minutes IS NOT NULL) OR
        mode IN ('one', 'flood')),
    CONSTRAINT tv_slots_position_unique UNIQUE (channel_id, position) DEFERRABLE INITIALLY DEFERRED
);

-- 4. Engine state and the generated timeline ---------------------------------
CREATE TABLE tv_channel_state (
    channel_id    UUID PRIMARY KEY REFERENCES tv_channels(id) ON DELETE CASCADE,
    -- `lisek_tv::EngineState`: slot cursor, per-slot enumerators, next start.
    state         JSONB NOT NULL DEFAULT '{}',
    built_until   TIMESTAMPTZ,
    built_version INTEGER NOT NULL DEFAULT 0,
    last_build_at TIMESTAMPTZ,
    last_error    TEXT,
    -- Set by anything that changes what the channel should play (a removed
    -- library item, a scan); the next builder pass refreshes the timeline.
    dirty         BOOLEAN NOT NULL DEFAULT true
);

CREATE TABLE tv_playout_items (
    id             BIGSERIAL PRIMARY KEY,
    channel_id     UUID NOT NULL REFERENCES tv_channels(id) ON DELETE CASCADE,
    starts_at      TIMESTAMPTZ NOT NULL,
    ends_at        TIMESTAMPTZ NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('content', 'pre', 'mid', 'post', 'tail', 'pad')),
    media_item_id  UUID REFERENCES media_items(id) ON DELETE SET NULL,
    filler_file_id UUID REFERENCES tv_filler_files(id) ON DELETE SET NULL,
    -- Where in the file this entry plays, in milliseconds.
    in_ms          INTEGER NOT NULL DEFAULT 0 CHECK (in_ms >= 0),
    out_ms         INTEGER NOT NULL CHECK (out_ms > in_ms),
    title          TEXT NOT NULL,
    subtitle       TEXT,
    slot_id        UUID REFERENCES tv_slots(id) ON DELETE SET NULL,
    -- Entries of one programme (a film and its ads) share a group; the guide
    -- shows the group as one programme.
    guide_group    INTEGER NOT NULL DEFAULT 0,
    in_guide       BOOLEAN NOT NULL DEFAULT true,
    CONSTRAINT tv_playout_items_range CHECK (ends_at > starts_at),
    CONSTRAINT tv_playout_items_start_unique UNIQUE (channel_id, starts_at)
);
-- "What plays at T": the last entry starting at or before T.
CREATE INDEX tv_playout_items_end ON tv_playout_items (channel_id, ends_at);
CREATE INDEX tv_playout_items_media ON tv_playout_items (media_item_id) WHERE media_item_id IS NOT NULL;

-- 5. API key scopes ---------------------------------------------------------
-- Existing keys keep the one thing they could do: read the statistics.
ALTER TABLE api_keys ADD COLUMN scopes TEXT[] NOT NULL DEFAULT '{stats}';

-- 6. Faza 20 IPTV channels in the lineup ------------------------------------
-- Overrides survive a refresh of the M3U (the refresh only rewrites the
-- columns it reads from the playlist).
ALTER TABLE live_channels
    ADD COLUMN enabled         BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN number_override TEXT,
    ADD COLUMN group_override  TEXT,
    ADD COLUMN logo_override   TEXT,
    ADD COLUMN failure_count   INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN last_failure_at TIMESTAMPTZ;
