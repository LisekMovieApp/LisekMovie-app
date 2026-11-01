-- Faza 21: the metadata editor for seasons, and the metadata language order.

-- A season's own edited fields (`name`, `overview`, `air_year`,
-- `poster`) are kept by provider refreshes.
ALTER TABLE seasons
    ADD COLUMN locked_fields      TEXT[] NOT NULL DEFAULT '{}',
    -- When an administrator replaced the season poster (ms): an older
    -- `seasonNN-poster.jpg` in the series folder no longer counts.
    ADD COLUMN poster_override_at BIGINT;

-- The order in which metadata languages are tried: language codes
-- (`pl`, `en` ...) plus `original` (the production's own language) and
-- `any`. Empty: the server's default language, then English, then the
-- original language.
ALTER TABLE server_settings
    ADD COLUMN metadata_languages TEXT[] NOT NULL DEFAULT '{}';
