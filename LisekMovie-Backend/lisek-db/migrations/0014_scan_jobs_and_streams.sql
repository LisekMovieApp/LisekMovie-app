-- Background library scans and stored ffprobe results.

-- File facts recorded by the scanner. A rescan compares them with the
-- disk to decide whether the stored probe is still valid; the upsert
-- clears `probed_at` when either one changes.
ALTER TABLE media_items
    ADD COLUMN file_size BIGINT,
    ADD COLUMN file_modified_at TIMESTAMPTZ,
    ADD COLUMN probed_at TIMESTAMPTZ,
    -- Set (with `probed_at`) when ffprobe could not read the file, so an
    -- unchanged broken file is not re-probed on every scan.
    ADD COLUMN probe_error TEXT,
    ADD COLUMN container TEXT,
    ADD COLUMN duration_seconds DOUBLE PRECISION,
    ADD COLUMN bitrate_kbps INTEGER;

CREATE INDEX media_items_needs_probe ON media_items (library_id) WHERE probed_at IS NULL;

-- One row per stream ffprobe reported. Replaced as a whole on every probe.
CREATE TABLE media_streams (
    media_item_id   UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    stream_index    INTEGER NOT NULL,
    stream_type     TEXT NOT NULL
        CHECK (stream_type IN ('video', 'audio', 'subtitle', 'attachment', 'data')),
    codec           TEXT,
    profile         TEXT,
    language        TEXT,
    title           TEXT,
    is_default      BOOLEAN NOT NULL DEFAULT false,
    is_forced       BOOLEAN NOT NULL DEFAULT false,
    width           INTEGER,
    height          INTEGER,
    bit_depth       INTEGER,
    pixel_format    TEXT,
    color_transfer  TEXT,
    color_primaries TEXT,
    color_space     TEXT,
    field_order     TEXT,
    frame_rate      DOUBLE PRECISION,
    channels        INTEGER,
    channel_layout  TEXT,
    sample_rate     INTEGER,
    bitrate_kbps    INTEGER,
    PRIMARY KEY (media_item_id, stream_index)
);

-- Scan queue. The server runs one job at a time; the partial unique index
-- makes "scan this library" idempotent while a scan is already pending.
CREATE TABLE scan_jobs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    library_id      UUID NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    requested_by    UUID REFERENCES users(id) ON DELETE SET NULL,
    status          TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    phase           TEXT NOT NULL DEFAULT 'queued'
        CHECK (phase IN ('queued', 'walking', 'saving', 'probing', 'enriching', 'done')),
    -- Progress within the current phase.
    phase_done      INTEGER NOT NULL DEFAULT 0,
    phase_total     INTEGER NOT NULL DEFAULT 0,
    files_found     INTEGER NOT NULL DEFAULT 0,
    movies_found    INTEGER NOT NULL DEFAULT 0,
    episodes_found  INTEGER NOT NULL DEFAULT 0,
    unrecognized    INTEGER NOT NULL DEFAULT 0,
    probed          INTEGER NOT NULL DEFAULT 0,
    probe_failed    INTEGER NOT NULL DEFAULT 0,
    metadata_matched INTEGER NOT NULL DEFAULT 0,
    images_saved    INTEGER NOT NULL DEFAULT 0,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ
);

CREATE UNIQUE INDEX scan_jobs_one_active_per_library
    ON scan_jobs (library_id) WHERE status IN ('queued', 'running');
CREATE INDEX scan_jobs_recent ON scan_jobs (created_at DESC);
CREATE INDEX scan_jobs_pending ON scan_jobs (created_at) WHERE status IN ('queued', 'running');
