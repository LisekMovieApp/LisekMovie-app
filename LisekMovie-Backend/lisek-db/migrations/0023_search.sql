-- Global search (Faza 11): trigram indexes (pg_trgm, 0013) for every
-- field the omnibox looks at, so `ILIKE '%...%'` stays an index scan.
-- Titles of films and series already have theirs (0013).

-- Words describing an item besides its title: the provider's original
-- title, genres and tags (keywords). IMMUTABLE so it can be indexed; the
-- search query calls it with the same expression to use the index.
-- PL/pgSQL on purpose: a SQL function would be inlined into the query,
-- which then no longer matches the indexed expression. COST tells the
-- planner that running it per row (a sequential scan) is expensive.
CREATE FUNCTION lisek_search_text(metadata jsonb) RETURNS text
LANGUAGE plpgsql IMMUTABLE PARALLEL SAFE COST 500
AS $$
BEGIN
    RETURN concat_ws(' ',
        metadata->'provider_data'->>'original_title',
        (SELECT string_agg(value, ' ')
         FROM jsonb_array_elements_text(
             CASE WHEN jsonb_typeof(metadata->'provider_data'->'genres') = 'array'
                  THEN metadata->'provider_data'->'genres' ELSE '[]'::jsonb END)),
        (SELECT string_agg(value, ' ')
         FROM jsonb_array_elements_text(
             CASE WHEN jsonb_typeof(metadata->'provider_data'->'tags') = 'array'
                  THEN metadata->'provider_data'->'tags' ELSE '[]'::jsonb END)));
END;
$$;

CREATE INDEX media_items_search_text_trgm
    ON media_items USING gin (lisek_search_text(metadata) gin_trgm_ops);

-- Episode titles from the provider (Faza 10).
CREATE INDEX media_items_episode_title_trgm
    ON media_items USING gin ((metadata->'provider_data'->'episode'->>'title') gin_trgm_ops)
    WHERE item_type = 'episode';

CREATE INDEX people_name_trgm ON people USING gin (name gin_trgm_ops);
CREATE INDEX collections_name_trgm ON collections USING gin (name gin_trgm_ops);
