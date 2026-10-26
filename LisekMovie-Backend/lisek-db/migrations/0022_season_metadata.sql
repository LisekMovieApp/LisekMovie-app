-- Season metadata (Faza 10): what the provider says about each season of
-- a series. Written by enrichment, read by the season cards and the
-- season page. `poster_url` is the source URL of the poster on disk
-- (`<series folder>/season01-poster.jpg`, or the local cache): a changed
-- URL is downloaded again.
ALTER TABLE seasons
    ADD COLUMN name       TEXT,
    ADD COLUMN overview   TEXT,
    ADD COLUMN poster_url TEXT,
    ADD COLUMN air_year   INTEGER;
