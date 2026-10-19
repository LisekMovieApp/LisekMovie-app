-- Relational series/seasons/episodes, typed columns and indexes for the
-- browse side (home feed, library pages, series detail).
--
-- Additive only. `media_items.metadata` stays the single value every
-- existing writer updates: the typed columns below are STORED generated
-- columns derived from it, so no write path can let them drift. The
-- series tables are maintained by statement-level triggers on
-- `media_items`, so scans, the metadata editor and library deletes all
-- keep them consistent without application code having to remember.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- 1. Typed columns ---------------------------------------------------------
-- `jsonb_typeof(...) = 'number'` keeps a malformed value (a string, JSON
-- null) from failing the cast; it simply yields NULL.
ALTER TABLE media_items
    ADD COLUMN year INTEGER GENERATED ALWAYS AS (
        CASE WHEN jsonb_typeof(metadata->'year') = 'number'
             THEN (metadata->>'year')::numeric::int END) STORED,
    ADD COLUMN season_number INTEGER GENERATED ALWAYS AS (
        CASE WHEN jsonb_typeof(metadata->'season') = 'number'
             THEN (metadata->>'season')::numeric::int END) STORED,
    ADD COLUMN episode_number INTEGER GENERATED ALWAYS AS (
        CASE WHEN jsonb_typeof(metadata->'episode') = 'number'
             THEN (metadata->>'episode')::numeric::int END) STORED,
    ADD COLUMN end_episode_number INTEGER GENERATED ALWAYS AS (
        CASE WHEN jsonb_typeof(metadata->'end_episode') = 'number'
             THEN (metadata->>'end_episode')::numeric::int END) STORED,
    ADD COLUMN overview TEXT GENERATED ALWAYS AS (
        metadata->'provider_data'->>'overview') STORED,
    -- Backs the A-Z filter with a plain btree lookup instead of ILIKE.
    ADD COLUMN title_initial TEXT GENERATED ALWAYS AS (upper(left(title, 1))) STORED,
    -- A random position per row. "Recommended"/"Popular" read a window of
    -- this index instead of sorting the whole table by random().
    ADD COLUMN shuffle_key DOUBLE PRECISION NOT NULL DEFAULT random();

-- 2. Series, seasons, episodes -------------------------------------------
-- A show is identified the same way the scanner already groups episodes:
-- by (library_id, title). `representative_item_id` is the most recently
-- added episode; the API keeps using its id for a series card, so every
-- existing URL (posters, /media/{id}) keeps working unchanged.
CREATE TABLE series (
    id                     UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    library_id             UUID NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    title                  TEXT NOT NULL,
    title_initial          TEXT GENERATED ALWAYS AS (upper(left(title, 1))) STORED,
    representative_item_id UUID REFERENCES media_items(id) ON DELETE SET NULL,
    episode_count          INTEGER NOT NULL DEFAULT 0,
    season_count           INTEGER NOT NULL DEFAULT 0,
    shuffle_key            DOUBLE PRECISION NOT NULL DEFAULT random(),
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_episode_added_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT series_library_title_unique UNIQUE (library_id, title)
);

CREATE TABLE seasons (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    series_id     UUID NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    season_number INTEGER NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT seasons_series_number_unique UNIQUE (series_id, season_number)
);

-- 1:1 extension of an episode's `media_items` row. Playback, watch state
-- and every other FK keep pointing at `media_items`; this table only adds
-- the relational placement. `created_at` mirrors the item's own value so
-- "newest episode" queries never need to join back.
CREATE TABLE episodes (
    media_item_id      UUID PRIMARY KEY REFERENCES media_items(id) ON DELETE CASCADE,
    series_id          UUID NOT NULL REFERENCES series(id) ON DELETE CASCADE,
    season_id          UUID NOT NULL REFERENCES seasons(id) ON DELETE CASCADE,
    season_number      INTEGER NOT NULL,
    episode_number     INTEGER NOT NULL,
    end_episode_number INTEGER,
    created_at         TIMESTAMPTZ NOT NULL
);

-- 3. Sync functions ---------------------------------------------------------
-- Links the given items (if they are numbered episodes) to their series
-- and season, creating either on first sight. Returns every series id the
-- items now belong to.
CREATE FUNCTION lisek_link_episodes(item_ids UUID[]) RETURNS UUID[]
LANGUAGE plpgsql AS $$
DECLARE
    touched UUID[];
BEGIN
    INSERT INTO series (library_id, title, created_at, last_episode_added_at)
    SELECT m.library_id, m.title, min(m.created_at), max(m.created_at)
    FROM media_items m
    WHERE m.id = ANY(item_ids) AND m.item_type = 'episode'
      AND m.season_number IS NOT NULL AND m.episode_number IS NOT NULL
    GROUP BY m.library_id, m.title
    ON CONFLICT (library_id, title) DO NOTHING;

    INSERT INTO seasons (series_id, season_number, created_at)
    SELECT s.id, m.season_number, min(m.created_at)
    FROM media_items m
    JOIN series s ON s.library_id = m.library_id AND s.title = m.title
    WHERE m.id = ANY(item_ids) AND m.item_type = 'episode'
      AND m.season_number IS NOT NULL AND m.episode_number IS NOT NULL
    GROUP BY s.id, m.season_number
    ON CONFLICT (series_id, season_number) DO NOTHING;

    INSERT INTO episodes (media_item_id, series_id, season_id, season_number,
                          episode_number, end_episode_number, created_at)
    SELECT m.id, s.id, se.id, m.season_number, m.episode_number, m.end_episode_number, m.created_at
    FROM media_items m
    JOIN series s ON s.library_id = m.library_id AND s.title = m.title
    JOIN seasons se ON se.series_id = s.id AND se.season_number = m.season_number
    WHERE m.id = ANY(item_ids) AND m.item_type = 'episode' AND m.episode_number IS NOT NULL
    ON CONFLICT (media_item_id) DO UPDATE SET
        series_id = EXCLUDED.series_id,
        season_id = EXCLUDED.season_id,
        season_number = EXCLUDED.season_number,
        episode_number = EXCLUDED.episode_number,
        end_episode_number = EXCLUDED.end_episode_number
    WHERE (episodes.series_id, episodes.season_id, episodes.episode_number, episodes.end_episode_number)
          IS DISTINCT FROM
          (EXCLUDED.series_id, EXCLUDED.season_id, EXCLUDED.episode_number, EXCLUDED.end_episode_number);

    -- An item edited into something that is no longer a numbered episode.
    DELETE FROM episodes e
    USING media_items m
    WHERE e.media_item_id = m.id AND m.id = ANY(item_ids)
      AND (m.item_type <> 'episode' OR m.season_number IS NULL OR m.episode_number IS NULL);

    SELECT coalesce(array_agg(DISTINCT e.series_id), '{}') INTO touched
    FROM episodes e WHERE e.media_item_id = ANY(item_ids);
    RETURN touched;
END;
$$;

-- Recomputes the denormalized counters and the representative episode,
-- and removes seasons/series left without episodes.
CREATE FUNCTION lisek_refresh_series(series_ids UUID[]) RETURNS void
LANGUAGE sql AS $$
    DELETE FROM seasons se
    WHERE se.series_id = ANY(series_ids)
      AND NOT EXISTS (SELECT 1 FROM episodes e WHERE e.season_id = se.id);

    DELETE FROM series s
    WHERE s.id = ANY(series_ids)
      AND NOT EXISTS (SELECT 1 FROM episodes e WHERE e.series_id = s.id);

    UPDATE series s SET
        episode_count = st.episode_count,
        season_count = st.season_count,
        last_episode_added_at = st.last_added,
        representative_item_id = st.representative,
        updated_at = now()
    FROM (
        SELECT e.series_id,
               count(*)::int AS episode_count,
               count(DISTINCT e.season_number)::int AS season_count,
               max(e.created_at) AS last_added,
               (array_agg(e.media_item_id ORDER BY e.created_at DESC, e.media_item_id DESC))[1]
                   AS representative
        FROM episodes e
        WHERE e.series_id = ANY(series_ids)
        GROUP BY e.series_id
    ) st
    WHERE s.id = st.series_id
      AND (s.episode_count, s.season_count, s.last_episode_added_at, s.representative_item_id)
          IS DISTINCT FROM
          (st.episode_count, st.season_count, st.last_added, st.representative);
$$;

-- Statement-level triggers with transition tables: a batched scan insert
-- of 500 files runs the sync once, not 500 times.
CREATE FUNCTION lisek_series_after_insert() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    ids UUID[];
BEGIN
    SELECT array_agg(id) INTO ids FROM new_rows WHERE item_type = 'episode';
    IF ids IS NOT NULL THEN
        PERFORM lisek_refresh_series(lisek_link_episodes(ids));
    END IF;
    RETURN NULL;
END;
$$;

CREATE FUNCTION lisek_series_after_update() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    ids UUID[];
    previous UUID[];
BEGIN
    -- Most updates (provider metadata, fallback flags, rescans of an
    -- unchanged file) touch nothing the series tables depend on.
    SELECT array_agg(n.id) INTO ids
    FROM new_rows n
    JOIN old_rows o ON o.id = n.id
    WHERE (n.item_type = 'episode' OR o.item_type = 'episode')
      AND (n.library_id, n.item_type, n.title, n.season_number, n.episode_number, n.end_episode_number)
          IS DISTINCT FROM
          (o.library_id, o.item_type, o.title, o.season_number, o.episode_number, o.end_episode_number);
    IF ids IS NULL THEN
        RETURN NULL;
    END IF;

    SELECT coalesce(array_agg(DISTINCT e.series_id), '{}') INTO previous
    FROM episodes e WHERE e.media_item_id = ANY(ids);
    PERFORM lisek_refresh_series(previous || lisek_link_episodes(ids));
    RETURN NULL;
END;
$$;

CREATE FUNCTION lisek_series_after_delete() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    touched UUID[];
BEGIN
    -- The episodes rows are already gone (FK cascade), so the series is
    -- found by the same (library_id, title) key the scanner groups by.
    SELECT array_agg(DISTINCT s.id) INTO touched
    FROM old_rows o
    JOIN series s ON s.library_id = o.library_id AND s.title = o.title
    WHERE o.item_type = 'episode';
    IF touched IS NOT NULL THEN
        PERFORM lisek_refresh_series(touched);
    END IF;
    RETURN NULL;
END;
$$;

CREATE TRIGGER media_items_series_insert
    AFTER INSERT ON media_items
    REFERENCING NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION lisek_series_after_insert();

CREATE TRIGGER media_items_series_update
    AFTER UPDATE ON media_items
    REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows
    FOR EACH STATEMENT EXECUTE FUNCTION lisek_series_after_update();

CREATE TRIGGER media_items_series_delete
    AFTER DELETE ON media_items
    REFERENCING OLD TABLE AS old_rows
    FOR EACH STATEMENT EXECUTE FUNCTION lisek_series_after_delete();

-- 4. Backfill from rows scanned before this migration ---------------------
SELECT lisek_refresh_series(lisek_link_episodes(
    ARRAY(SELECT id FROM media_items WHERE item_type = 'episode')));

-- 5. Indexes --------------------------------------------------------------
-- Movies: one partial index per browse ordering, so each home-feed row is
-- an index scan that stops after LIMIT rows.
CREATE INDEX media_items_movie_recent
    ON media_items (created_at DESC, id DESC) WHERE item_type = 'movie';
CREATE INDEX media_items_movie_library_recent
    ON media_items (library_id, created_at DESC, id DESC) WHERE item_type = 'movie';
CREATE INDEX media_items_movie_library_title
    ON media_items (library_id, title, id) WHERE item_type = 'movie';
CREATE INDEX media_items_movie_library_initial
    ON media_items (library_id, title_initial, title, id) WHERE item_type = 'movie';
CREATE INDEX media_items_movie_year
    ON media_items (year DESC NULLS LAST, created_at DESC, id DESC) WHERE item_type = 'movie';
CREATE INDEX media_items_movie_shuffle
    ON media_items (shuffle_key) WHERE item_type = 'movie';
CREATE INDEX media_items_movie_library_shuffle
    ON media_items (library_id, shuffle_key) WHERE item_type = 'movie';
-- Admin browse list and title search (ILIKE '%...%').
CREATE INDEX media_items_library_title ON media_items (library_id, title);
CREATE INDEX media_items_title_trgm ON media_items USING gin (title gin_trgm_ops);
CREATE INDEX media_items_fallback ON media_items (title) WHERE metadata_fallback;

-- Series cards are addressed by their representative episode's id, so
-- that id is the tie-breaker the queries sort by.
CREATE INDEX series_recent ON series (last_episode_added_at DESC, representative_item_id DESC);
CREATE INDEX series_library_recent
    ON series (library_id, last_episode_added_at DESC, representative_item_id DESC);
CREATE INDEX series_library_initial ON series (library_id, title_initial, title);
CREATE INDEX series_shuffle ON series (shuffle_key);
CREATE INDEX series_library_shuffle ON series (library_id, shuffle_key);
CREATE INDEX series_title_trgm ON series USING gin (title gin_trgm_ops);
-- Card id -> series lookups, and the ON DELETE SET NULL check that runs
-- for every deleted media item.
CREATE INDEX series_representative ON series (representative_item_id);

CREATE INDEX episodes_series_order ON episodes (series_id, season_number, episode_number, media_item_id);
CREATE INDEX episodes_series_recent ON episodes (series_id, created_at DESC, media_item_id DESC);
CREATE INDEX episodes_season ON episodes (season_id);

-- Foreign keys that cascade from media_items had no index on the
-- referencing side, so deleting a library ran one sequential scan of each
-- of these tables per deleted item.
CREATE INDEX browser_notification_events_item ON browser_notification_events (item_id);
CREATE INDEX transcode_kills_item ON transcode_kills (item_id);
CREATE INDEX transcode_kills_session ON transcode_kills (session_id);
CREATE INDEX user_media_state_item ON user_media_state (item_id);
CREATE INDEX webhook_delivery_attempts_delivery ON webhook_delivery_attempts (delivery_id);

ANALYZE media_items;
ANALYZE series;
ANALYZE seasons;
ANALYZE episodes;
