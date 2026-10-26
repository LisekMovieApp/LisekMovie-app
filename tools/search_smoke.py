"""Global search (Faza 11) and the alpha picker (Faza 12) for the
disposable-database integration suite:
`GET /api/search` groups (films, series, episodes, people, collections),
short and over-long queries, authentication and parental controls.

Runs after the series checks, so the catalogue holds the smoke film, the
"Lisek Show" series with its titled first episode, the cast from the
people checks and the collections from the collections checks."""

from urllib.parse import quote


def check_search(request, ok_json, sql, token, other_token, other_id):
    def search(q, tok=token):
        return ok_json(f"/api/search?q={quote(q)}", token=tok)

    groups = ("movies", "series", "episodes", "people", "collections")

    # Too short: every group empty, no database work.
    short = search("a")
    assert all(short[g] == [] for g in groups), short
    assert request("/api/search?q=" + "x" * 101, token=token)[0] == 400
    assert request("/api/search?q=lisek")[0] == 401

    # Films by title (a fragment from the middle, any case).
    movie_id, movie_title = sql(
        "SELECT id || '|' || title FROM media_items WHERE item_type = 'movie' "
        "AND length(title) >= 4 ORDER BY created_at LIMIT 1").split("|", 1)
    fragment = movie_title[1:4].upper()
    found = search(fragment)
    assert movie_id in [m["id"] for m in found["movies"]], (fragment, found["movies"])

    # Series by title, episodes by their own title.
    found = search("lisek sh")
    assert [s["title"] for s in found["series"]][:1] == ["Lisek Show"], found["series"]
    found = search("najwyższy")
    episode = found["episodes"][0]
    assert (episode["series_title"], episode["season"], episode["episode"], episode["title"]) == \
        ("Lisek Show", 1, 1, "Najwyższy czas"), found["episodes"]
    assert episode["thumb_url"] == f"/api/media/{episode['id']}/thumb"

    # People and collections by name.
    person = sql("SELECT name FROM people ORDER BY name LIMIT 1")
    if person:
        found = search(person)
        assert person in [p["name"] for p in found["people"]], found["people"]
    collection = sql("SELECT name FROM collections WHERE length(name) >= 2 ORDER BY name LIMIT 1")
    if collection:
        found = search(collection)
        assert collection in [c["title"] for c in found["collections"]], found["collections"]

    # LIKE wildcards are literal.
    assert all(search("%%")[g] == [] for g in groups)

    # Parental controls: an account limited to the films library sees no series.
    films = sql("SELECT id FROM libraries WHERE library_type = 'movies' LIMIT 1")
    try:
        ok_json(f"/api/admin/users/{other_id}/restrictions", "PUT", {"libraries": [films]}, token)
        found = search("lisek sh", other_token)
        assert found["series"] == [] and found["episodes"] == [], found
        assert search("najwyższy", other_token)["episodes"] == []
        assert movie_id in [m["id"] for m in search(fragment, other_token)["movies"]]
    finally:
        ok_json(f"/api/admin/users/{other_id}/restrictions", "PUT", {}, token)


def check_letters(request, ok_json, token, library_id, other_token, other_id):
    """Alpha picker (Faza 12): `GET /api/libraries/{id}/letters` points each
    letter at the position of its first card in the unfiltered list."""
    items = ok_json(f"/api/libraries/{library_id}/items?page_size=200", token=token)["items"]
    letters = ok_json(f"/api/libraries/{library_id}/letters", token=token)["letters"]
    assert letters, "a library with titles has letters"
    for entry in letters:
        title = items[entry["index"]]["title"]
        first = title[0].upper()
        assert entry["letter"] == (first if "A" <= first <= "Z" else "#") or not first.isascii(), (entry, title)
    assert [e["index"] for e in letters] == sorted(e["index"] for e in letters), letters
    desc = ok_json(f"/api/libraries/{library_id}/letters?sort=title_desc", token=token)["letters"]
    assert desc[0]["index"] == 0 and len(desc) == len(letters), desc
    assert request(f"/api/libraries/{library_id}/letters")[0] == 401
    try:
        ok_json(f"/api/admin/users/{other_id}/restrictions", "PUT", {"libraries": []}, token)
        assert request(f"/api/libraries/{library_id}/letters", token=other_token)[0] == 404
    finally:
        ok_json(f"/api/admin/users/{other_id}/restrictions", "PUT", {}, token)
