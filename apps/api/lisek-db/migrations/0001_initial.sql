-- Initial schema: users, libraries, metadata provider registry, media items.
-- Single migration mechanism (sqlx migrate) from day one — see architecture plan.

CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- Local accounts. Full auth (tokens, API keys, sessions) lands in Faza 3 —
-- this table only needs to support creating the first admin in the
-- First-Run Wizard for now.
CREATE TABLE users (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    is_admin      BOOLEAN NOT NULL DEFAULT false,
    ui_language   TEXT NOT NULL DEFAULT 'en',
    email         TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Libraries: navigation tabs. sort_order is set and reordered globally by
-- the admin (not per-user) — see architecture plan.
CREATE TABLE libraries (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name          TEXT NOT NULL,
    library_type  TEXT NOT NULL,
    sort_order    INTEGER NOT NULL,
    root_paths    TEXT[] NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Global metadata provider registry: enabled state + settings (e.g. API keys).
CREATE TABLE provider_config (
    provider_key  TEXT PRIMARY KEY,
    enabled       BOOLEAN NOT NULL DEFAULT true,
    settings      JSONB NOT NULL DEFAULT '{}'
);

-- Per (library, item type) provider ordering — lets two libraries of the
-- same type use completely different provider configurations.
CREATE TABLE library_provider_config (
    library_id      UUID NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    item_type       TEXT NOT NULL,
    provider_order  TEXT[] NOT NULL,
    PRIMARY KEY (library_id, item_type)
);

-- Media items.
CREATE TABLE media_items (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    library_id        UUID NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    item_type         TEXT NOT NULL,
    title             TEXT NOT NULL,
    file_path         TEXT NOT NULL,
    metadata_locked   BOOLEAN NOT NULL DEFAULT false,
    forced_provider   TEXT REFERENCES provider_config(provider_key),
    metadata          JSONB NOT NULL DEFAULT '{}',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);
