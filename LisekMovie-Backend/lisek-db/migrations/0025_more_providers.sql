-- Faza 16: rows for the new metadata provider extensions (IMDb, OMDb,
-- AniList, aniDB, aniSearch, IMVDb, VGMdb, TheAudioDB). Disabled until an
-- administrator turns them on in Admin -> Plugins, so existing libraries
-- keep matching exactly as before; placed after every provider already
-- configured, in lisek_metadata::PROVIDERS order.
INSERT INTO provider_config (provider_key, enabled, settings, priority)
SELECT p.provider_key, false, '{}', base.max_priority + p.position
FROM (VALUES
    ('imdb', 1),
    ('omdb', 2),
    ('anilist', 3),
    ('anidb', 4),
    ('anisearch', 5),
    ('imvdb', 6),
    ('vgmdb', 7),
    ('audiodb', 8)
) AS p(provider_key, position)
CROSS JOIN (SELECT COALESCE(MAX(priority), 0) AS max_priority FROM provider_config) AS base
ON CONFLICT (provider_key) DO NOTHING;
