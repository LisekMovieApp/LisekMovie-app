-- Chapters (Faza 9): the scenes a Matroska/MP4 file lists, read by
-- ffprobe (`-show_chapters`) together with the streams.
--
-- Times are in playback time (the container's start_time subtracted), so
-- a scene opens the player at exactly `start_seconds`. `chapter_index` is
-- the position in start order; the detail page addresses a chapter's
-- thumbnail by it. Thumbnails are made on first request and cached under
-- `config/chapters/<item id>/`.
CREATE TABLE media_chapters (
    media_item_id UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    chapter_index INTEGER NOT NULL CHECK (chapter_index >= 0),
    start_seconds DOUBLE PRECISION NOT NULL CHECK (start_seconds >= 0),
    end_seconds   DOUBLE PRECISION,
    title         TEXT,
    PRIMARY KEY (media_item_id, chapter_index)
);

-- Files probed before this migration have no chapters read yet: the next
-- scan probes them once more (only them), then this stays true.
ALTER TABLE media_items ADD COLUMN chapters_probed BOOLEAN NOT NULL DEFAULT false;
