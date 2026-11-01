"""Faza 21 for the disposable-database integration suite: the software
fallback when the configured GPU cannot be opened, the season editor and
an episode's own score and directors, the metadata language order, and the
metadata cascade (a TMDB mock without a Polish synopsis, poster or genres;
an OMDb mock that has them).

`smoke_test.py` points the TMDB and OMDb plugins at `start_mock_metadata`'s
server (`LISEK_PROVIDER_URL_TMDB`, `LISEK_PROVIDER_URL_OMDB`) before the
server starts."""

import json
import shutil
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from phase20_smoke import raw, scan
from profiles_smoke import png

IMDB_ID = "tt7654321"
TMDB_ID = 7654321
OMDB_POSTER = b"\xff\xd8\xff\xe0OMDB-POSTER-KASKADA"
ENGLISH = "A waterfall nobody had ever seen."


def start_mock_metadata():
    """TMDB (`/tmdb/...`) and OMDb (`/omdb/`) stand-ins for one film,
    "Kaskada" (2021). TMDB has no Polish synopsis, no genres in Polish and
    no poster; OMDb has a synopsis, genres and a poster. Returns
    (server, base_url, requests seen)."""
    seen = []

    class Handler(BaseHTTPRequestHandler):
        def reply(self, payload, content_type="application/json", status=200):
            body = payload if isinstance(payload, bytes) else json.dumps(payload).encode()
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            url = urllib.parse.urlsplit(self.path)
            query = dict(urllib.parse.parse_qsl(url.query))
            path = url.path
            seen.append((path, query.get("language")))
            base = f"http://127.0.0.1:{self.server.server_port}"
            polish = query.get("language", "").startswith("pl")
            if path.startswith("/tmdb"):
                path = path[len("/tmdb"):]
                if path == "/search/movie":
                    found = "kaskada" in query.get("query", "").lower()
                    results = [{"id": TMDB_ID, "title": "Kaskada", "release_date": "2021-03-01",
                                "overview": "" if polish else ENGLISH, "poster_path": None,
                                "backdrop_path": None}] if found else []
                    return self.reply({"page": 1, "results": results, "total_results": len(results)})
                if path == f"/movie/{TMDB_ID}/images":
                    return self.reply({"posters": [], "backdrops": [], "logos": []})
                if path == f"/movie/{TMDB_ID}":
                    return self.reply({
                        "id": TMDB_ID, "title": "Kaskada", "release_date": "2021-03-01",
                        "overview": "" if polish else ENGLISH, "poster_path": None, "backdrop_path": None,
                        "belongs_to_collection": None, "genres": [] if polish else [{"id": 18, "name": "Drama"}],
                        "imdb_id": IMDB_ID, "original_language": "fr", "original_title": "La Cascade",
                        "tagline": "", "runtime": 0, "production_companies": [],
                        "release_dates": {"results": []}, "credits": {"cast": [], "crew": []},
                        "keywords": {"keywords": []},
                    })
                return self.reply({"status_message": "not found"}, status=404)
            if path.startswith("/omdb"):
                if path.startswith("/omdb/img"):
                    return self.reply(OMDB_POSTER, "image/jpeg")
                if "s" in query:
                    found = "kaskada" in query["s"].lower()
                    if not found:
                        return self.reply({"Response": "False", "Error": "Movie not found!"})
                    return self.reply({"Search": [{"Title": "Kaskada", "Year": "2021", "imdbID": IMDB_ID,
                                                   "Type": "movie", "Poster": f"{base}/omdb/img/k.jpg"}],
                                       "totalResults": "1", "Response": "True"})
                if query.get("i") == IMDB_ID or "kaskada" in query.get("t", "").lower():
                    return self.reply({
                        "Title": "Kaskada", "Year": "2021", "Rated": "N/A", "Runtime": "95 min",
                        "Genre": "Drama, Mystery", "Plot": "OMDb plot", "Production": "N/A",
                        "Poster": f"{base}/omdb/img/k.jpg", "imdbRating": "7.1",
                        "imdbID": IMDB_ID, "Type": "movie", "Response": "True",
                    })
                return self.reply({"Response": "False", "Error": "Incorrect IMDb ID."})
            return self.reply(b"not found", "text/plain", 404)

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}", seen


def check_hardware_fallback(request, ok_json, sql, token, client):
    """VAAPI on a render node that does not exist (the i5-5300U case): the
    film still plays, transcoded on the CPU, and the hardware page shows
    the device as broken."""
    settings = ok_json("/api/admin/playback/settings", token=token)
    before = dict(settings)
    settings["hardware_acceleration"] = "vaapi"
    settings["hardware_device_path"] = "/dev/dri/renderD199"
    ok_json("/api/admin/playback/settings", "PUT", settings, token)
    try:
        assert ok_json("/api/admin/playback/settings", token=token)["hardware_acceleration"] == "vaapi"
        item = sql("SELECT id FROM media_items WHERE file_path LIKE '%Lisek Story (2020).mkv'")
        started = ok_json(f"/api/media/{item}/playback", "POST",
                          {"client": client, "max_height": 240, "max_bitrate_kbps": 800}, token)
        try:
            assert started["mode"] == "transcode", started
            status, body, _ = request(started["stream_url"])
            assert status == 200 and b"#EXTM3U" in body, (status, body[:200])
        finally:
            ok_json(f"/api/playback/{started['session_id']}", "DELETE", token=token)
        health = ok_json("/api/admin/hardware", token=token)["health"]
        broken = [h for h in health if h["id"].startswith("vaapi") and not h["ok"]]
        assert broken and broken[0]["error"], health
    finally:
        ok_json("/api/admin/playback/settings", "PUT", before, token)
    # Saving the settings forgets the verdicts (the administrator fixed it).
    assert ok_json("/api/admin/hardware", token=token)["health"] == []


def check_season_and_episode_editor(request, ok_json, sql, base, token, other_token):
    e1 = sql("SELECT id FROM media_items WHERE file_path LIKE '%Lisek Show S01E01%'")
    e2 = sql("SELECT id FROM media_items WHERE file_path LIKE '%Lisek Show S01E02%'")
    library_id = sql(f"SELECT library_id FROM media_items WHERE id = '{e1}'")

    season = ok_json(f"/api/admin/media/{e1}/seasons/1", token=token)
    assert season["season"] == 1 and season["episode_count"] == 2, season
    saved = ok_json(f"/api/admin/media/{e1}/seasons/1", "PUT", {
        "name": "Pierwszy sezon", "overview": "Opis sezonu", "air_year": 2024, "locked_fields": []}, token)
    assert sorted(saved["auto_locked"]) == ["air_year", "name", "overview"], saved
    assert request(f"/api/admin/media/{e1}/seasons/1", "PUT", {"name": "x", "air_year": 99, "locked_fields": []},
                   token)[0] == 400
    assert request(f"/api/admin/media/{e1}/seasons/1", "PUT", {"name": "x", "locked_fields": []},
                   other_token)[0] == 403
    assert request(f"/api/admin/media/{e1}/seasons/9", token=token)[0] == 404

    # An episode's own score and directors; the series' directors.
    saved = ok_json(f"/api/admin/media/{e2}", "PUT", {
        "scope": "episode", "episode_rating": 7.46, "episode_directors": ["Jan Reżyser", " ", "Anna Kamera"],
        "locked_fields": []}, token)
    assert {"episode_rating", "episode_directors"} <= set(saved["auto_locked"]), saved
    assert request(f"/api/admin/media/{e2}", "PUT", {"scope": "episode", "episode_rating": 11,
                                                     "locked_fields": []}, token)[0] == 400
    # As the editor sends it: the item's current title and locks.
    current = ok_json(f"/api/admin/media/{e1}", token=token)
    saved = ok_json(f"/api/admin/media/{e1}", "PUT", {
        "scope": "series", "title": current["title"], "rating": 8, "directors": ["Reżyser Serialu"],
        "locked_fields": current["locked_fields"]}, token)
    assert {"rating", "directors"} <= set(saved["auto_locked"]), saved

    page = ok_json(f"/api/media/{e1}/seasons/1/episodes", token=token)
    scan(request, ok_json, token, library_id)
    page = ok_json(f"/api/media/{e1}/seasons/1/episodes", token=token)
    assert (page["name"], page["overview"], page["air_year"]) == ("Pierwszy sezon", "Opis sezonu", 2024), page
    second = next(e for e in page["episodes"] if e["id"] == e2)
    assert second["rating"] == 7.5 and second["directors"] == ["Jan Reżyser", "Anna Kamera"], second
    item = ok_json(f"/api/admin/media/{e2}", token=token)
    assert item["episode_rating"] == 7.5 and item["rating"] == 8 and item["directors"] == ["Reżyser Serialu"], item
    details = ok_json(f"/api/media/{e1}/details", token=token)
    assert [d["name"] for d in details["directors"]] == ["Reżyser Serialu"], details["directors"]
    # A missing rating keeps it, null clears it.
    ok_json(f"/api/admin/media/{e2}", "PUT", {"scope": "episode", "locked_fields": ["episode_rating"]}, token)
    assert ok_json(f"/api/admin/media/{e2}", token=token)["episode_rating"] == 7.5
    ok_json(f"/api/admin/media/{e2}", "PUT", {"scope": "episode", "episode_rating": None,
                                              "locked_fields": ["episode_rating"]}, token)
    assert ok_json(f"/api/admin/media/{e2}", token=token)["episode_rating"] is None

    # The season poster.
    status, body = raw(base, f"/api/admin/media/{e1}/seasons/1/poster", token, png(600, 900, (10, 200, 90)), "image/png")
    assert status == 200, body
    url = json.loads(body)["url"]
    status, poster, _ = request(url, token=token)
    assert status == 200 and poster[:3] == b"\xff\xd8\xff", (status, poster[:8])
    status, again, _ = request(f"/api/media/{e2}/seasons/1/poster", token=token)
    assert status == 200 and again == poster
    assert sql(f"SELECT 'poster' = ANY(locked_fields) FROM seasons WHERE id = "
               f"(SELECT season_id FROM episodes WHERE media_item_id = '{e1}')") == "t"
    assert raw(base, f"/api/admin/media/{e1}/seasons/1/poster", other_token, png(10, 10, (0, 0, 0)), "image/png")[0] == 403


def check_languages(request, ok_json, token, other_token):
    default = ok_json("/api/admin/metadata/languages", token=token)
    assert default["languages"] == [] and default["effective"][-1] == "original", default
    saved = ok_json("/api/admin/metadata/languages", "PUT",
                    {"languages": ["PL", "en", "original", "any", "pl"]}, token)
    assert saved == {"languages": ["pl", "en", "original", "any"], "effective": ["pl", "en", "original", "any"]}, saved
    status, body, _ = request("/api/admin/metadata/languages", "PUT", {"languages": ["polski"]}, token)
    assert status == 400 and json.loads(body)["error_code"] == "invalid_input", body
    assert request("/api/admin/metadata/languages", token=other_token)[0] == 403
    assert request("/api/admin/metadata/languages", "PUT", {"languages": []}, other_token)[0] == 403


def check_cascade(request, ok_json, sql, token, root, movie_path, seen):
    """TMDB first: its Polish answer has no synopsis, poster or genres. The
    synopsis comes from TMDB in English (the next language), the poster
    and genres from OMDb (the next plugin, same IMDb id)."""
    folder = root / "kaskada"
    folder.mkdir()
    shutil.copy(movie_path, folder / "Kaskada (2021).mp4")
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
        library = ok_json("/api/admin/libraries", "POST", {"name": "Kaskada", "library_type": "movies",
                                                           "root_paths": [str(folder)],
                                                           "save_metadata_in_media_folder": False}, token)
        scan(request, ok_json, token, library["id"])
        item = sql(f"SELECT id FROM media_items WHERE library_id = '{library['id']}'")
        data = json.loads(sql(f"SELECT metadata->'provider_data' FROM media_items WHERE id = '{item}'"))
        assert data["overview"] == ENGLISH, data
        assert data["language_from"] == {"overview": "en"}, data
        assert data["genres"] == ["Drama", "Mystery"], data
        assert data["filled_from"]["genres"] == "omdb" and data["filled_from"]["poster"] == "omdb", data
        assert data["tmdb_id"] == TMDB_ID and data["imdb_id"] == IMDB_ID, data
        assert ("/tmdb/movie/%d" % TMDB_ID, "en") in seen, seen
        status, poster, _ = request(f"/api/media/{item}/poster", token=token)
        assert status == 200 and poster == OMDB_POSTER, (status, poster[:20])
    finally:
        for key, (enabled, priority) in before.items():
            ok_json(f"/api/admin/providers/{key}", "PUT", {"enabled": enabled, "priority": priority}, token)
        for key in ("tmdb", "omdb"):
            request(f"/api/admin/providers/{key}/key", "DELETE", token=token)
        ok_json("/api/admin/metadata/languages", "PUT", {"languages": []}, token)
