"""Series pages (Faza 10) for the disposable-database integration suite:
a TV library with two episodes in `Season 01/`, the season cards, the
season page, whole-season flags, season posters and episode thumbnails,
and parental controls."""

import json
import shutil
import time


def check_series(request, ok_json, sql, token, other_token, other_id, root, movie_path, has_ffmpeg):
    season_dir = root / "shows" / "Lisek Show" / "Season 01"
    season_dir.mkdir(parents=True)
    suffix = movie_path.suffix
    for n in (1, 2):
        shutil.copy(movie_path, season_dir / f"Lisek Show S01E0{n}{suffix}")
    library = ok_json("/api/admin/libraries", "POST", {
        "name": "Seriale smoke", "library_type": "tv_shows",
        "root_paths": [str(root / "shows")], "save_metadata_in_media_folder": True}, token)
    status, body, _ = request(f"/api/admin/libraries/{library['id']}/scan", "POST", {}, token)
    assert status == 202, (status, body)
    job = json.loads(body)
    for _ in range(600):
        job = ok_json(f"/api/admin/scan-jobs/{job['id']}", token=token)
        if job["status"] in ("completed", "failed"):
            break
        time.sleep(.1)
    assert job["status"] == "completed" and job["episodes_found"] == 2, job
    e1 = sql("SELECT id FROM media_items WHERE file_path LIKE '%S01E01%'")
    e2 = sql("SELECT id FROM media_items WHERE file_path LIKE '%S01E02%'")
    sql("UPDATE media_items SET metadata = jsonb_set(metadata, '{provider_data}', "
        "coalesce(metadata->'provider_data', '{}'::jsonb) || "
        "'{\"episode\": {\"title\": \"Najwyższy czas\", \"overview\": \"Lis...\", \"runtime_minutes\": 51, \"rating\": 7.5}}'::jsonb) "
        f"WHERE id = '{e1}'")
    sql(f"UPDATE seasons SET name = 'Sezon 1', air_year = 2023 WHERE season_number = 1")

    cards = ok_json(f"/api/media/{e2}/season-cards", token=token)["items"]
    assert [(c["season"], c["name"], c["episode_count"], c["unwatched_count"], c["play_id"]) for c in cards] == [
        (1, "Sezon 1", 2, 2, e1)], cards
    assert cards[0]["poster_url"] == f"/api/media/{e2}/seasons/1/poster"

    page = ok_json(f"/api/media/{e1}/seasons/1/episodes", token=token)
    assert [(e["id"], e["title"]) for e in page["episodes"]] == [(e1, "Najwyższy czas"), (e2, None)], page
    assert page["episodes"][0]["rating"] == 7.5 and page["episodes"][0]["thumb_url"] == f"/api/media/{e1}/thumb"
    assert request(f"/api/media/{e1}/seasons/9/episodes", token=token)[0] == 404

    # Whole season watched, then favourite.
    assert request(f"/api/media/{e1}/seasons/1/state", "PUT", {"watched": True}, token)[0] == 204
    page = ok_json(f"/api/media/{e1}/seasons/1/episodes", token=token)
    assert all(e["watched"] for e in page["episodes"])
    assert ok_json(f"/api/media/{e1}/season-cards", token=token)["items"][0]["unwatched_count"] == 0
    assert ok_json(f"/api/media/{e1}/season-cards", token=other_token)["items"][0]["unwatched_count"] == 2, "per viewer"
    assert request(f"/api/media/{e1}/seasons/1/state", "PUT", {"favorite": True}, token)[0] == 204
    assert ok_json(f"/api/media/{e1}/season-cards", token=token)["items"][0]["favorite"] is True
    assert request(f"/api/media/{e1}/seasons/9/state", "PUT", {"watched": True}, token)[0] == 404

    # No artwork yet: the season poster falls back to the series poster (none: 404).
    assert request(f"/api/media/{e1}/seasons/1/poster?token={token}")[0] == 404
    (root / "shows" / "Lisek Show" / "season01-poster.jpg").write_bytes(b"\xff\xd8season")
    status, body, _ = request(f"/api/media/{e1}/seasons/1/poster?token={token}")
    assert status == 200 and body == b"\xff\xd8season", (status, body)
    if has_ffmpeg:
        status, body, _ = request(f"/api/media/{e1}/thumb?token={token}")
        assert status == 200 and body[:2] == b"\xff\xd8", (status, body[:20])
    assert sorted(p.name for p in season_dir.iterdir()) == [f"Lisek Show S01E01{suffix}", f"Lisek Show S01E02{suffix}"], \
        "season folders hold only videos"

    # Parental controls: only the films library.
    films = sql("SELECT id FROM libraries WHERE library_type = 'movies' LIMIT 1")
    try:
        ok_json(f"/api/admin/users/{other_id}/restrictions", "PUT", {"libraries": [films]}, token)
        assert request(f"/api/media/{e1}/season-cards", token=other_token)[0] == 404
        assert request(f"/api/media/{e1}/seasons/1/episodes", token=other_token)[0] == 404
        assert request(f"/api/media/{e1}/seasons/1/poster?token={other_token}")[0] == 404
        assert request(f"/api/media/{e1}/thumb?token={other_token}")[0] == 404
        assert request(f"/api/media/{e1}/seasons/1/state", "PUT", {"watched": False}, other_token)[0] == 404
    finally:
        ok_json(f"/api/admin/users/{other_id}/restrictions", "PUT", {}, token)
