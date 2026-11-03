"""Faza 24 for the disposable-database integration suite: Fix match to
another production replaces the whole cast and crew (no "Frankenstein"
of two films), and people credited nowhere any more are removed.

Uses the TMDB mock of `phase21_smoke` (the "Kaskada" film and the wrong
"Robin Hood" one, each with its own cast and director)."""

from phase21_smoke import ROBIN_ID, TMDB_ID


def cast_of(ok_json, token, item):
    return sorted((c["name"], c["role"]) for c in ok_json(f"/api/media/{item}/people", token=token)["items"])


def directors_of(ok_json, token, item):
    return [d["name"] for d in ok_json(f"/api/media/{item}/details", token=token)["directors"]]


def people_named(sql, *names):
    quoted = ",".join("'" + n.replace("'", "''") + "'" for n in names)
    return int(sql(f"SELECT count(*) FROM people WHERE name IN ({quoted})"))


def check_fix_match_replaces_cast(request, ok_json, sql, token):
    item = sql("SELECT id FROM media_items WHERE file_path LIKE '%Kaskada (2021).mp4'")
    providers = ok_json("/api/admin/providers", token=token)
    before = {p["provider_key"]: (p["enabled"], p["priority"]) for p in providers}
    try:
        for p in providers:
            if p["enabled"]:
                ok_json(f"/api/admin/providers/{p['provider_key']}", "PUT", {"enabled": False, "priority": p["priority"]}, token)
        ok_json("/api/admin/providers/tmdb/key", "PUT", {"api_key": "smoke-tmdb"}, token)
        ok_json("/api/admin/providers/tmdb", "PUT", {"enabled": True, "priority": 0}, token)

        # The wrong film, with a director typed by hand for it.
        ok_json(f"/api/media/{item}/identify", "POST", {"provider": "tmdb", "provider_id": str(ROBIN_ID)}, token)
        assert cast_of(ok_json, token, item) == [("Matthew Porretta", "Actor"), ("Robin Director", "Director")]
        current = ok_json(f"/api/admin/media/{item}", token=token)
        ok_json(f"/api/admin/media/{item}", "PUT", {
            "scope": "item", "title": current["title"], "original_title": current["original_title"],
            "year": current["year"], "overview": current["overview"], "locked_fields": current["locked_fields"],
            "metadata_locked": current["metadata_locked"], "directors": ["Robin Director"]}, token)
        assert directors_of(ok_json, token, item) == ["Robin Director"]

        # Fix match to the right film: only its cast and director remain.
        answer = ok_json(f"/api/media/{item}/identify", "POST", {"provider": "tmdb", "provider_id": str(TMDB_ID)}, token)
        assert answer["items_updated"] == 1, answer
        assert cast_of(ok_json, token, item) == [("Kaskada Director", "Director"), ("Kaskada Star", "Actor")]
        assert directors_of(ok_json, token, item) == ["Kaskada Director"]
        edited = ok_json(f"/api/admin/media/{item}", token=token)
        assert edited["directors"] == [] and "directors" not in edited["locked_fields"], edited
        assert int(sql(f"SELECT count(*) FROM media_people WHERE media_item_id = '{item}'")) == 2

        # Garbage collection: the wrong film's people were credited nowhere else.
        assert people_named(sql, "Matthew Porretta", "Robin Director") == 0
        assert people_named(sql, "Kaskada Star", "Kaskada Director") == 2
    finally:
        sql(f"UPDATE media_items SET metadata_locked = false WHERE id = '{item}'")
        for key, (enabled, priority) in before.items():
            ok_json(f"/api/admin/providers/{key}", "PUT", {"enabled": enabled, "priority": priority}, token)
        request("/api/admin/providers/tmdb/key", "DELETE", token=token)
