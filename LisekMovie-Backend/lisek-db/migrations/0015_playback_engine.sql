-- Facts the HLS playback engine needs from ffprobe, and a cache of
-- keyframe positions for files whose container has no usable index.

-- ffprobe's format.start_time: the timestamp playback time zero maps to
-- (large for MPEG-TS/M2TS). NULL for probes stored before this migration;
-- playback probes such files again once.
ALTER TABLE media_items ADD COLUMN start_time_seconds DOUBLE PRECISION;

-- Dolby Vision profile of a video stream (5, 7, 8 ...). Profile 5 has no
-- HDR10-compatible base layer, so it cannot be copied to clients without
-- Dolby Vision.
ALTER TABLE media_streams ADD COLUMN dovi_profile INTEGER;

-- Keyframe timestamps (seconds, the file's own timeline) of the primary
-- video stream, found by reading the whole file with ffprobe. Copying
-- video into HLS needs them to cut segments; Matroska files carry them in
-- their Cues and are not stored here. Valid while the file still has the
-- size and modification time it was scanned at.
CREATE TABLE media_keyframes (
    media_item_id    UUID PRIMARY KEY REFERENCES media_items(id) ON DELETE CASCADE,
    file_size        BIGINT,
    file_modified_at TIMESTAMPTZ,
    keyframes        DOUBLE PRECISION[] NOT NULL,
    scanned_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
