CREATE TABLE games (
    game_key TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT false
);
INSERT INTO games(game_key,name) VALUES ('flappy-bird','Flappy Fox');
CREATE TABLE game_scores (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    game_key TEXT NOT NULL REFERENCES games(game_key) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    score INTEGER NOT NULL CHECK(score BETWEEN 0 AND 100000),
    achieved_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX game_scores_ranking ON game_scores(game_key,score DESC);
