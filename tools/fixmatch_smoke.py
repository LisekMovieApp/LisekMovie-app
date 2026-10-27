"""Fix Match end to end (Faza 17) for the disposable-database integration
suite: an IMDb id typed for a series, picked and applied must still be
there after a reload — title, provider data and the poster — also when the
series folder holds an older `poster.jpg` (from Jellyfin, or an earlier
match) that the new poster cannot replace.

The IMDb provider talks to a local mock of its suggestion endpoint
(`LISEK_PROVIDER_URL_IMDB`, set by `smoke_test.py` before the server
starts)."""

import json
import os
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

IMDB_ID = "tt37543164"
NEW_POSTER = b"\xff\xd8\xff\xe0NEW-POSTER-ROJBERS"
OLD_POSTER = b"\xff\xd8\xff\xe0OLD-JELLYFIN-POSTER"


def start_mock_imdb():
    """A stand-in for `v3.sg.media-imdb.com`: suggestions for one id and one
    title, and the poster image. Returns (server, base_url)."""

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            path = urllib.parse.unquote(self.path)
            base = f"http://127.0.0.1:{self.server.server_port}"
            if path.startswith("/suggestion/"):
                query = path.rsplit("/", 1)[1].removesuffix(".json")
                entries = []
                if query in (IMDB_ID, "rojbers"):
                    entries.append({
                        "id": IMDB_ID, "l": "Rojbers", "y": 2025, "qid": "tvSeries",
                        "s": "Aktor Jeden, Aktor Dwa",
                        "i": {"imageUrl": f"{base}/img/rojbers._V1_.jpg", "width": 1, "height": 1},
                    })
                body = json.dumps({"d": entries, "q": query, "v": 1}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
            elif path.startswith("/img/"):
                body = NEW_POSTER
                self.send_response(200)
                self.send_header("Content-Type", "image/jpeg")
            else:
                body = b"not found"
                self.send_response(404)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}"


def check_fix_match_identify(request, ok_json, sql, token, root):
    series_dir = root / "shows" / "Lisek Show"
    e1 = sql("SELECT id FROM media_items WHERE file_path LIKE '%S01E01%'")
    e2 = sql("SELECT id FROM media_items WHERE file_path LIKE '%S01E02%'")
    library_id = sql(f"SELECT library_id FROM media_items WHERE id = '{e1}'")

    imdb = next(p for p in ok_json("/api/admin/providers", token=token) if p["provider_key"] == "imdb")
    ok_json("/api/admin/providers/imdb", "PUT", {"enabled": True, "priority": imdb["priority"]}, token)
    try:
        # An artwork file the server did not write and cannot replace: the
        # library keeps artwork in the server's cache only.
        old_poster = series_dir / "poster.jpg"
        old_poster.write_bytes(OLD_POSTER)
        an_hour_ago = time.time() - 3600
        os.utime(old_poster, (an_hour_ago, an_hour_ago))
        sql(f"UPDATE libraries SET save_metadata_in_media_folder = false WHERE id = '{library_id}'")

        found = ok_json("/api/providers/search?" + urllib.parse.urlencode(
            {"query": IMDB_ID, "kind": "series", "provider": "imdb"}), token=token)
        assert found["id_kind"] == "imdb", found
        picked = found["results"][0]
        assert (picked["provider_id"], picked["title"], picked["matched_by"]) == (IMDB_ID, "Rojbers", "id"), found

        result = ok_json(f"/api/media/{e1}/identify", "POST",
                         {"provider": "imdb", "provider_id": picked["provider_id"]}, token)
        assert (result["title"], result["items_updated"], result["image_saved"]) == ("Rojbers", 2, True), result

        # "F5": everything read again from the server.
        for item in (e1, e2):
            detail = ok_json(f"/api/media/{item}", token=token)
            assert detail["title"] == "Rojbers", detail
            stored = json.loads(sql(f"SELECT metadata::text FROM media_items WHERE id = '{item}'"))
            assert stored["provider"] == "imdb" and stored["provider_data"]["imdb_id"] == IMDB_ID, stored
            assert sql(f"SELECT metadata_locked || '|' || forced_provider FROM media_items WHERE id = '{item}'") == "true|imdb"
        assert sql("SELECT count(*) FROM series WHERE title = 'Rojbers'") == "1"
        assert sql("SELECT count(*) FROM series WHERE title = 'Lisek Show'") == "0"
        status, body, headers = request(f"/api/media/{e1}/poster?token={token}")
        assert status == 200 and body == NEW_POSTER, ("the new poster, not the folder's old one", status, body[:40])
        # The browser revalidates instead of keeping the old picture for an hour.
        assert "no-cache" in headers.get("Cache-Control", ""), headers.get("Cache-Control")
        etag = headers.get("ETag")
        assert etag, headers
        status, body, _ = request(f"/api/media/{e1}/poster?token={token}", headers={"If-None-Match": etag})
        assert status == 304 and body == b"", (status, body[:20])
        # The scanner sees the files again: the fix stays.
        status, body, _ = request(f"/api/admin/libraries/{library_id}/scan", "POST", {}, token)
        assert status == 202, (status, body)
        job = json.loads(body)
        for _ in range(600):
            job = ok_json(f"/api/admin/scan-jobs/{job['id']}", token=token)
            if job["status"] in ("completed", "failed"):
                break
            time.sleep(.1)
        assert job["status"] == "completed", job
        assert ok_json(f"/api/media/{e1}", token=token)["title"] == "Rojbers"

        # A failing provider is an error the modal can show, not a fake success.
        status, body, _ = request(f"/api/media/{e1}/identify", "POST",
                                  {"provider": "imdb", "provider_id": "tt0000001"}, token)
        assert status == 404 and b"provider_item_not_found" in body, (status, body)
    finally:
        ok_json("/api/admin/providers/imdb", "PUT", {"enabled": False, "priority": imdb["priority"]}, token)
