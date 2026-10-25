-- Collections (box sets) and per-account access restrictions.

-- 1. Collections --------------------------------------------------------------
-- A collection comes from a metadata provider (TMDB `belongs_to_collection`:
-- "Star Wars Collection", "Harry Potter Collection") and is identified by
-- that provider's id, so every film of the series links to the same row no
-- matter which library or scan found it. Members are plain `media_items`
-- rows (movies). A collection whose last member is gone is removed by
-- `lisek_db::collections::prune_empty` after scans.
CREATE TABLE collections (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    provider     TEXT NOT NULL,
    provider_id  TEXT NOT NULL,
    name         TEXT NOT NULL,
    -- Source URLs of the artwork downloaded into
    -- `config/metadata/collections/<id>/`; a changed URL is downloaded again.
    poster_url   TEXT,
    backdrop_url TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT collections_provider_unique UNIQUE (provider, provider_id)
);

CREATE TABLE collection_items (
    collection_id UUID NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    media_item_id UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    PRIMARY KEY (collection_id, media_item_id)
);
CREATE INDEX collection_items_item ON collection_items (media_item_id);

-- 2. Age rating -----------------------------------------------------------------
-- Minimum viewer age from the provider's certification (TMDB: PG-13 -> 13,
-- FSK 12 -> 12, TV-MA -> 17), as `provider_data.age_rating`. A top-level
-- `metadata.age_rating` (set by an administrator) wins over it. NULL means
-- unrated.
ALTER TABLE media_items
    ADD COLUMN age_rating INTEGER GENERATED ALWAYS AS (
        CASE
            WHEN jsonb_typeof(metadata->'age_rating') = 'number'
                THEN (metadata->>'age_rating')::numeric::int
            WHEN jsonb_typeof(metadata->'provider_data'->'age_rating') = 'number'
                THEN (metadata->'provider_data'->>'age_rating')::numeric::int
        END) STORED;

-- 3. Per-account restrictions ------------------------------------------------------
-- No row = no restrictions. Administrators are never restricted, whatever
-- a row says (enforced where the session is resolved).
--   libraries       NULL = every library; otherwise only these
--                   (a deleted library simply stops matching).
--   max_age_rating  NULL = no age limit; otherwise items rated above it
--                   are hidden.
--   block_unrated   with an age limit, also hide items without a rating.
CREATE TABLE user_restrictions (
    user_id        UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    libraries      UUID[],
    max_age_rating INTEGER CHECK (max_age_rating BETWEEN 0 AND 21),
    block_unrated  BOOLEAN NOT NULL DEFAULT false,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The one visibility rule every catalog query shares. A plain SQL
-- expression marked IMMUTABLE, so the planner inlines it into the calling
-- query (no per-row function call); with every restriction NULL/false it
-- folds to `true`.
CREATE FUNCTION lisek_visible(
    library_id     UUID,
    age_rating     INTEGER,
    libraries      UUID[],
    max_age_rating INTEGER,
    block_unrated  BOOLEAN
) RETURNS BOOLEAN
LANGUAGE sql IMMUTABLE PARALLEL SAFE
AS $$
    SELECT ($3 IS NULL OR $1 = ANY ($3))
       AND ($4 IS NULL
            OR CASE WHEN $2 IS NULL THEN NOT coalesce($5, false)
                    ELSE $2 <= $4 END)
$$;
