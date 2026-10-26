-- Cast and crew (Faza 8): people credited on films and series.
--
-- One `people` row per person, identified by the provider's own id (only
-- TMDB has people today), so the same actor links every title they appear
-- in. The credits come with each enrichment; biography and dates are read
-- from the provider the first time someone opens the person's page
-- (`details_updated_at`), not during scans.

CREATE TABLE people (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name               TEXT NOT NULL,
    tmdb_id            BIGINT NOT NULL,
    imdb_id            TEXT,
    biography          TEXT,
    birth_place        TEXT,
    birth_date         DATE,
    death_date         DATE,
    -- Source URL of the profile photo downloaded into
    -- `config/metadata/people/<id>/profile.jpg`; a changed URL is
    -- downloaded again, NULL means no photo on disk.
    profile_url        TEXT,
    details_updated_at TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT people_tmdb_unique UNIQUE (tmdb_id)
);

-- Who did what on an item. Series credits are stored on every episode (the
-- provider answers once per series, so enrichment writes the same list for
-- each); filmography queries fold episodes back into one series card.
--   role            'Actor', 'Director' or 'Writer'
--   character_name  the part an actor plays
--   sort_order      billing order within the role, 0 first
CREATE TABLE media_people (
    media_item_id  UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    person_id      UUID NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    role           TEXT NOT NULL CHECK (role IN ('Actor', 'Director', 'Writer')),
    character_name TEXT,
    sort_order     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (media_item_id, person_id, role)
);

-- The primary key serves "credits of an item"; this one "titles of a
-- person" and the orphan sweep after scans.
CREATE INDEX media_people_person ON media_people (person_id, media_item_id);
