-- Playback telemetry: where each session last was and how smoothly it
-- played. Filled by `POST /api/telemetry/playback-progress`.
ALTER TABLE playback_sessions
    ADD COLUMN position_seconds DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (position_seconds >= 0),
    -- Rebuffering events and time, summed over the session (seeks excluded).
    ADD COLUMN stall_count INTEGER NOT NULL DEFAULT 0 CHECK (stall_count >= 0),
    ADD COLUMN stall_seconds DOUBLE PRECISION NOT NULL DEFAULT 0 CHECK (stall_seconds >= 0),
    -- Latest values the player reported.
    ADD COLUMN dropped_frames INTEGER,
    ADD COLUMN bandwidth_kbps INTEGER;
