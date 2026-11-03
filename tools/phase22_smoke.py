"""Faza 22 for the disposable-database integration suite: an item renamed
by hand is fetched again by its stored ids and never searched by its new
title (the "Różowa Pantera" became "Robin Hood" case), the ids shown and
changed in the metadata editor, and a title the first plugin left
untranslated taken from the next metadata language.

Uses the TMDB and OMDb mocks of `phase21_smoke` and the "Kaskada" film
that `phase21_smoke.check_cascade` matched."""

import json

from phase20_smoke import scan
from phase21_smoke import IMDB_ID, ROBIN_ID, SEARCHES, TMDB_ID


def editor_payload(item, **changes):
    """What the editor sends: the item's current fields and locks."""
    payload = {"scope": "item", "title": item["title"], "original_title": item["original_title"],
               "year": item["year"], "overview": item["overview"], "locked_fields": item["locked_fields"]}
    payload.update(changes)
    return payload


def provider_data(sql, item):
    return json.loads(sql(f"SELECT metadata->'provider_data' FROM media_items WHERE id = '{item}'"))


def check_manual_edits_keep_the_identity(request, ok_json, sql, token, other_token, seen):
    item = sql("SELECT id FROM media_items WHERE file_path LIKE '%Kaskada (2021).mp4'")
    library_id = sql(f"SELECT library_id FROM media_items WHERE id = '{item}'")
    providers = ok_json("/api/admin/providers", token=token)
    before = {p["provider_key"]: (p["enabled"], p["priority"]) for p in providers}
    try:
        for p in providers:
            if p["enabled"]:
                ok_json(f"/api/admin/providers/{p['provider_key']}", "PUT", {"enabled": False, "priority": p["priority"]}, token)
        for n, key in enumerate(("tmdb", "omdb")):
            ok_json(f"/api/admin/providers/{key}/key", "PUT", {"api_key": f"smoke-{key}"}, token)
            ok_json(f"/api/admin/providers/{key}", "PUT", {"enabled": True, "priority": n}, token)
        ok_json("/api/admin/metadata/languages", "PUT", {"languages": ["pl", "en", "original"]}, token)

        # The editor shows the ids the film was matched with.
        loaded = ok_json(f"/api/admin/media/{item}", token=token)
        assert loaded["provider_ids"] == {"tmdb": str(TMDB_ID), "imdb": IMDB_ID}, loaded["provider_ids"]
        assert loaded["provider"] == "tmdb", loaded

        # Renamed by hand: the title and the ids are locked.
        saved = ok_json(f"/api/admin/media/{item}", "PUT", editor_payload(loaded, title="Nowe Przygody Kaskady"), token)
        assert sorted(saved["auto_locked"]) == ["provider_ids", "title"], saved
        assert "refetched" not in saved, saved

        # Fetched again (the editor's button) and rescanned: by the ids only.
        SEARCHES.clear()
        seen.clear()
        summary = ok_json(f"/api/admin/media/{item}/refresh", "POST", {}, token)
        assert summary == {"items": 1, "matched": 1}, summary
        scan(request, ok_json, token, library_id)
        assert SEARCHES == [], SEARCHES
        assert not any(path == f"/tmdb/movie/{ROBIN_ID}" for path, _ in seen), seen
        data = provider_data(sql, item)
        assert data["tmdb_id"] == TMDB_ID and data["imdb_id"] == IMDB_ID, data
        assert sql(f"SELECT title FROM media_items WHERE id = '{item}'") == "Nowe Przygody Kaskady"
        # The Polish answer only had the original title: the English one wins.
        assert data["title"] == "The Cascade" and data["language_from"]["title"] == "en", data
        assert request(f"/api/admin/media/{item}/refresh", "POST", {}, other_token)[0] == 403

        # The ids changed in the editor: fetched again from the new film,
        # and only the ids the administrator kept stay.
        current = ok_json(f"/api/admin/media/{item}", token=token)
        saved = ok_json(f"/api/admin/media/{item}", "PUT",
                        editor_payload(current, provider_ids={"tmdb": f" {ROBIN_ID} "}), token)
        assert saved["refetched"] == {"items": 1, "matched": 1}, saved
        data = provider_data(sql, item)
        assert data["tmdb_id"] == ROBIN_ID and "imdb_id" not in data, data
        assert data["overview"] == "Robin Hood", data
        assert ok_json(f"/api/admin/media/{item}", token=token)["provider_ids"] == {"tmdb": str(ROBIN_ID)}

        for bad in ({"imdb": "1234"}, {"tmdb": "12a"}, {"netflix": "1"}):
            status, body, _ = request(f"/api/admin/media/{item}", "PUT", editor_payload(current, provider_ids=bad), token)
            assert status == 400 and json.loads(body)["error_code"] == "invalid_provider_id", (bad, body)

        # Back to the right film.
        saved = ok_json(f"/api/admin/media/{item}", "PUT",
                        editor_payload(current, provider_ids={"tmdb": str(TMDB_ID), "imdb": IMDB_ID.upper()}), token)
        assert saved["refetched"]["matched"] == 1, saved
        data = provider_data(sql, item)
        assert (data["tmdb_id"], data["imdb_id"]) == (TMDB_ID, IMDB_ID), data

        # No ids and edited by hand: never searched by the new title.
        ok_json(f"/api/admin/media/{item}", "PUT", editor_payload(current, provider_ids={}), token)
        SEARCHES.clear()
        assert ok_json(f"/api/admin/media/{item}/refresh", "POST", {}, token)["matched"] == 0
        assert SEARCHES == [], SEARCHES
    finally:
        for key, (enabled, priority) in before.items():
            ok_json(f"/api/admin/providers/{key}", "PUT", {"enabled": enabled, "priority": priority}, token)
        for key in ("tmdb", "omdb"):
            request(f"/api/admin/providers/{key}/key", "DELETE", token=token)
        ok_json("/api/admin/metadata/languages", "PUT", {"languages": []}, token)
