-- Faza 25: named API keys for scripts and automations (Discord and
-- Telegram bots, Home Assistant, own dashboards), replacing the single
-- stats key. Only a hash of each key is stored; the key itself is shown
-- once when it is created.
CREATE TABLE api_keys (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name         TEXT NOT NULL CHECK (char_length(name) BETWEEN 1 AND 100),
    token_hash   TEXT NOT NULL UNIQUE,
    -- The key's first characters, so an administrator can tell keys apart
    -- (`lsk_AbC1…`).
    prefix       TEXT NOT NULL DEFAULT '',
    created_by   UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ
);

-- The bots' key made before this keeps working, under a name.
INSERT INTO api_keys (name, token_hash, created_at)
SELECT 'Statystyki (bot)', key_hash, created_at FROM stats_api_key;

DROP TABLE stats_api_key;
