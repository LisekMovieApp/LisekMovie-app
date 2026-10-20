-- Trickplay: seek-bar preview sprite sheets generated in the background.
-- The JPEG sheets live under the server's config directory
-- (`trickplay/<item id>/sheet-NNN.jpg`); this row says what they contain.
-- `source_size`/`source_modified_at` copy the file facts the sheets were
-- made from: when the scanner records different ones the item is
-- generated again. A failed item is retried only after its file changes
-- (or an administrator asks for it).
CREATE TABLE media_trickplay (
    item_id            UUID PRIMARY KEY REFERENCES media_items(id) ON DELETE CASCADE,
    status             TEXT NOT NULL CHECK (status IN ('ready', 'failed')),
    interval_seconds   INTEGER NOT NULL DEFAULT 0 CHECK (interval_seconds >= 0),
    tile_width         INTEGER NOT NULL DEFAULT 0,
    tile_height        INTEGER NOT NULL DEFAULT 0,
    grid_columns       INTEGER NOT NULL DEFAULT 0,
    grid_rows          INTEGER NOT NULL DEFAULT 0,
    thumbnail_count    INTEGER NOT NULL DEFAULT 0,
    sheet_count        INTEGER NOT NULL DEFAULT 0,
    source_size        BIGINT,
    source_modified_at TIMESTAMPTZ,
    error              TEXT,
    generated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- "Next episode" reads `episodes_series_order` (0013): no new index needed.
