-- Faza 5: a simple global display order for the Admin Panel's plugins tab,
-- layered on top of `provider_config`'s existing `enabled`/`settings`
-- columns (Faza 1 scaffold). This is deliberately just a flat integer, not
-- a reordering of `library_provider_config`'s per-(library, item_type)
-- `provider_order` array — that mechanism still exists for a library to
-- override the order later; `priority` is only the single list + numbers
-- the Admin Panel shows before any libraries exist to configure.
ALTER TABLE provider_config ADD COLUMN priority INTEGER NOT NULL DEFAULT 0;

-- Seed the two providers built into the core app itself (as opposed to a
-- future external plugin via lisek-plugin-api, Faza 7). Enabled by default
-- with empty settings — the admin fills in an API key via the Admin Panel
-- before either can actually make a request; a provider stays listed and
-- toggleable even before it's configured, so the admin sees both providers
-- exist without having to "add" them first.
INSERT INTO provider_config (provider_key, enabled, settings, priority) VALUES
    ('tmdb', true, '{}', 0),
    ('tvdb', true, '{}', 1)
ON CONFLICT (provider_key) DO NOTHING;
