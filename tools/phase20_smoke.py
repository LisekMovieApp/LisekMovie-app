"""Faza 20 for the disposable-database integration suite: the metadata
editor (episodes, whole series, edits surviving a rescan, artwork from a
file and from a link), the player's quality presets, watching together
(two WebSocket members through Node) and Live TV (an M3U list and an XMLTV
guide from a local web server, a live HLS stream from FFmpeg)."""

import json
import subprocess
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from profiles_smoke import png


def raw(base, path, token, body, content_type, method="PUT"):
    req = urllib.request.Request(base + path, method=method, data=body,
                                 headers={"Authorization": f'LisekMovie Token="{token}"', "Content-Type": content_type})
    try:
        response = urllib.request.urlopen(req, timeout=60)
    except urllib.error.HTTPError as error:
        response = error
    return response.status, response.read()


def start_files(files):
    """A tiny web server for `files` ({path: (content_type, bytes)}). Returns (server, base)."""

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            entry = files.get(self.path.split("?")[0])
            if entry is None:
                self.send_response(404)
                self.end_headers()
                return
            content_type, body = entry
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, f"http://127.0.0.1:{server.server_port}"


def scan(request, ok_json, token, library_id):
    status, body, _ = request(f"/api/admin/libraries/{library_id}/scan", "POST", {}, token)
    assert status == 202, (status, body)
    job = json.loads(body)
    for _ in range(600):
        job = ok_json(f"/api/admin/scan-jobs/{job['id']}", token=token)
        if job["status"] in ("completed", "failed"):
            break
        time.sleep(.1)
    assert job["status"] == "completed", job


def check_editor(request, ok_json, sql, base, token, other_token):
    e1 = sql("SELECT id FROM media_items WHERE file_path LIKE '%Lisek Show S01E01%'")
    e2 = sql("SELECT id FROM media_items WHERE file_path LIKE '%Lisek Show S01E02%'")
    series_id = sql(f"SELECT series_id FROM episodes WHERE media_item_id = '{e1}'")
    library_id = sql(f"SELECT library_id FROM media_items WHERE id = '{e1}'")
    series_title = sql(f"SELECT title FROM series WHERE id = '{series_id}'")

    # One episode: its own title, not the series it belongs to.
    item = ok_json(f"/api/admin/media/{e1}", token=token)
    assert item["series_id"] == series_id and item["series_title"] == series_title, item
    saved = ok_json(f"/api/admin/media/{e1}", "PUT", {
        "scope": "episode", "episode_title": "Odcinek z edytora", "episode_overview": "Opis od administratora",
        "episode_air_date": "2024-05-06", "locked_fields": []}, token)
    assert "episode_title" in saved["auto_locked"] and saved["updated"] == 1, saved
    status, body, _ = request(f"/api/admin/media/{e1}", "PUT", {
        "scope": "episode", "episode_air_date": "6.05.2024", "locked_fields": []}, token)
    assert status == 400 and json.loads(body)["error_code"] == "invalid_air_date", body
    page = ok_json(f"/api/media/{e1}/seasons/1/episodes", token=token)
    first = next(e for e in page["episodes"] if e["id"] == e1)
    assert (first["title"], first["overview"], first["air_date"]) == ("Odcinek z edytora", "Opis od administratora", "2024-05-06"), first
    assert sql(f"SELECT title FROM media_items WHERE id = '{e1}'") == series_title

    # The whole series: renamed in place, every episode, kept after a rescan.
    saved = ok_json(f"/api/admin/media/{e1}", "PUT", {
        "scope": "series", "title": "Seria z edytora", "year": 2023, "overview": "Opis serialu",
        "locked_fields": []}, token)
    assert saved["updated"] == 2 and "title" in saved["auto_locked"], saved
    assert sql(f"SELECT title FROM series WHERE id = '{series_id}'") == "Seria z edytora"
    assert sql(f"SELECT count(DISTINCT series_id) FROM episodes WHERE media_item_id IN ('{e1}','{e2}')") == "1"
    scan(request, ok_json, token, library_id)
    assert sql(f"SELECT DISTINCT title FROM media_items WHERE id IN ('{e1}','{e2}')") == "Seria z edytora"
    assert sql(f"SELECT series_id FROM episodes WHERE media_item_id = '{e2}'") == series_id
    page = ok_json(f"/api/media/{e1}/seasons/1/episodes", token=token)
    assert next(e for e in page["episodes"] if e["id"] == e1)["title"] == "Odcinek z edytora"
    assert request(f"/api/admin/media/{e1}", "PUT", {"scope": "series", "title": "x", "locked_fields": []}, other_token)[0] == 403

    # Artwork: an upload replaces the series poster for every episode.
    status, body = raw(base, f"/api/admin/media/{e1}/artwork/poster", token, png(900, 1350, (20, 120, 220)), "image/png")
    assert status == 200, body
    answer = json.loads(body)
    assert answer["items"] == 2 and answer["url"].startswith(f"/api/media/{e1}/poster?v="), answer
    status, poster, headers = request(answer["url"], token=token)
    assert status == 200 and poster[:3] == b"\xff\xd8\xff", (status, poster[:8])
    status, sibling, _ = request(f"/api/media/{e2}/poster", token=token)
    assert status == 200 and sibling == poster
    assert sql(f"SELECT 'poster' = ANY(locked_fields) FROM media_items WHERE id = '{e2}'") == "t"
    # From a link (logos stay PNG); refused links and files.
    server, files_base = start_files({"/logo.png": ("image/png", png(400, 120, (250, 250, 250)))})
    try:
        status, body = raw(base, f"/api/admin/media/{e1}/artwork/logo", token,
                           json.dumps({"url": f"{files_base}/logo.png"}).encode(), "application/json")
        assert status == 200, body
        status, logo, headers = request(json.loads(body)["url"], token=token)
        assert status == 200 and headers["Content-Type"] == "image/png" and logo[:4] == b"\x89PNG"
        status, body = raw(base, f"/api/admin/media/{e1}/artwork/backdrop", token,
                           json.dumps({"url": f"{files_base}/missing.png"}).encode(), "application/json")
        assert status == 502 and json.loads(body)["error_code"] == "artwork_download_failed", body
    finally:
        server.shutdown()
    status, body = raw(base, f"/api/admin/media/{e1}/artwork/backdrop", token, b'{"url": "file:///etc/passwd"}', "application/json")
    assert status == 400 and json.loads(body)["error_code"] == "artwork_invalid_url", body
    status, body = raw(base, f"/api/admin/media/{e1}/artwork/poster", token, b"<svg xmlns='http://www.w3.org/2000/svg'/>", "image/svg+xml")
    assert status == 400 and json.loads(body)["error_code"] == "artwork_invalid_image", body
    assert raw(base, f"/api/admin/media/{e1}/artwork/banner", token, b"x", "image/png")[0] == 400
    # An episode's still.
    status, body = raw(base, f"/api/admin/media/{e2}/artwork/thumb", token, png(1280, 720, (200, 50, 50)), "image/png")
    assert status == 200 and json.loads(body)["items"] == 1, body
    status, thumb, _ = request(f"/api/media/{e2}/thumb", token=token)
    assert status == 200 and thumb[:3] == b"\xff\xd8\xff"
    assert raw(base, f"/api/admin/media/{e1}/artwork/poster", other_token, png(10, 10, (0, 0, 0)), "image/png")[0] == 403


def check_quality(ok_json, sql, token, client):
    """The player's "480p (2 Mbps)": the server scales down."""
    item = sql("SELECT id FROM media_items WHERE file_path LIKE '%Lisek Story (2020).mkv'")
    started = ok_json(f"/api/media/{item}/playback", "POST", {"client": client, "max_height": 240, "max_bitrate_kbps": 600}, token)
    try:
        assert started["mode"] == "transcode", started
        assert "resolution_above_requested" in started["reasons"], started["reasons"]
        assert started["delivered"]["height"] <= 240 and not started["delivered"]["video_copied"], started["delivered"]
    finally:
        ok_json(f"/api/playback/{started['session_id']}", "DELETE", token=token)


def check_syncplay(request, ok_json, sql, base, token, other_token, node):
    movie = sql("SELECT id FROM media_items WHERE item_type = 'movie' LIMIT 1")
    room = ok_json("/api/syncplay/rooms", "POST", {"item_id": movie, "position": 5}, token)
    assert len(room["code"]) == 6 and room["link"] == f"/together/{room['code']}", room
    info = ok_json(f"/api/syncplay/rooms/{room['code'].lower()}", token=other_token)
    assert info["item_id"] == movie and info["members"] == [] and not info["playing"] and info["position"] == 5, info
    assert request("/api/syncplay/rooms/ZZZZZZ", token=token)[0] == 404
    assert request("/api/syncplay/rooms", "POST", {"item_id": "00000000-0000-0000-0000-000000000000"}, token)[0] == 404
    if not node:
        return
    probe = subprocess.run([node, str(Path(__file__).with_name("syncplay_probe.mjs").resolve())],
                           input=json.dumps({"base": base, "a": token, "b": other_token, "code": room["code"]}),
                           capture_output=True, text=True, timeout=60)
    assert probe.returncode == 0, (probe.returncode, probe.stdout, probe.stderr)
    result = json.loads(probe.stdout.strip().splitlines()[-1])
    assert result["joined_b"]["position"] == 5 and len(result["joined_b"]["members"]) == 2, result
    assert result["b_got"] == {"type": "command", "action": "seek", "position": 120, "by": "test-admin"}, result
    assert result["a_got"]["action"] == "pause" and result["a_got"]["by"] == "other", result
    assert result["state"]["playing"] is False and result["state"]["position"] == 125, result
    assert result["bad_code"] == {"type": "error", "code": "room_not_found"}, result


def guide_xml(now):
    def stamp(t):
        return t.strftime("%Y%m%d%H%M%S +0000")
    start = now.replace(minute=0, second=0, microsecond=0)
    programmes = "".join(
        f'<programme start="{stamp(start + timedelta(hours=h))}" stop="{stamp(start + timedelta(hours=h + 1))}" channel="lisek1.pl">'
        f"<title>Audycja {h}</title><desc>Opis &amp; więcej</desc></programme>" for h in range(-1, 4))
    return (f'<?xml version="1.0" encoding="UTF-8"?><tv><channel id="lisek1.pl"><display-name>Lisek TV</display-name></channel>'
            f"{programmes}</tv>").encode()


def check_live_tv(request, ok_json, sql, base, token, other_token, root, ffmpeg):
    files = {"/logo.png": ("image/png", png(200, 100, (240, 160, 40))), "/guide.xml": ("application/xml", guide_xml(datetime.now(timezone.utc)))}
    if ffmpeg:
        stream = root / "live.ts"
        subprocess.run([ffmpeg, "-v", "error", "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=25:duration=12",
                        "-f", "lavfi", "-i", "sine=frequency=440:duration=12", "-c:v", "libx264", "-preset", "ultrafast",
                        "-g", "25", "-c:a", "aac", "-f", "mpegts", str(stream)], check=True)
        files["/live.ts"] = ("video/mp2t", stream.read_bytes())
    server, files_base = start_files(files)
    try:
        files["/list.m3u"] = ("audio/x-mpegurl", (
            "#EXTM3U\n"
            f'#EXTINF:-1 tvg-id="lisek1.pl" tvg-logo="{files_base}/logo.png" group-title="Smoke" tvg-chno="1",Lisek TV\n'
            f"{files_base}/live.ts\n"
            '#EXTINF:-1 tvg-id="dead",Martwy kanał\nhttp://127.0.0.1:9/dead.ts\n'
            "#EXTINF:-1,Plik\nfile:///etc/passwd\n").encode())
        assert request("/api/admin/libraries", "POST", {"name": "TV", "library_type": "live_tv", "root_paths": [],
                                                         "m3u_url": "ftp://x/list.m3u"}, token)[0] == 400
        library = ok_json("/api/admin/libraries", "POST", {
            "name": "Telewizja smoke", "library_type": "live_tv", "root_paths": [],
            "m3u_url": f"{files_base}/list.m3u", "xmltv_url": f"{files_base}/guide.xml"}, token)
        for _ in range(100):
            source = next(s for s in ok_json("/api/admin/livetv", token=token) if s["library_id"] == library["id"])
            if source["refreshed_at"]:
                break
            time.sleep(.1)
        assert source["last_error"] is None and source["channel_count"] == 2 and source["program_count"] == 5, source
        assert request(f"/api/admin/libraries/{library['id']}/scan", "POST", {}, token)[0] == 400
        assert all(lib["id"] != library["id"] for lib in ok_json("/api/libraries", token=token)), "no poster grid"

        assert ok_json("/api/livetv/status", token=other_token) == {"available": True, "channel_count": 2}
        channels = ok_json("/api/livetv/channels", token=token)
        lisek = next(c for c in channels if c["name"] == "Lisek TV")
        dead = next(c for c in channels if c["name"] == "Martwy kanał")
        assert lisek["now"]["title"] == "Audycja 0" and lisek["next"]["title"] == "Audycja 1", lisek
        assert lisek["group_title"] == "Smoke" and lisek["number"] == "1" and lisek["logo_url"], lisek
        guide = ok_json("/api/livetv/guide?hours=6", token=token)
        programs = next(c for c in guide["channels"] if c["id"] == lisek["id"])["programs"]
        assert programs[0]["description"] == "Opis & więcej" and len(programs) >= 4, programs
        status, logo, headers = request(lisek["logo_url"] + f"?token={token}")
        assert status == 200 and headers["Content-Type"] == "image/png" and logo[:4] == b"\x89PNG"

        # Library restrictions hide Live TV like any other library.
        other_id = sql("SELECT id FROM users WHERE username = 'other'")
        sql(f"INSERT INTO user_restrictions (user_id, libraries) VALUES ('{other_id}', ARRAY[]::uuid[]) "
            "ON CONFLICT (user_id) DO UPDATE SET libraries = ARRAY[]::uuid[]")
        assert ok_json("/api/livetv/status", token=other_token)["available"] is False
        assert request(f"/api/livetv/channels/{lisek['id']}/play", "POST", {}, other_token)[0] == 404
        sql(f"DELETE FROM user_restrictions WHERE user_id = '{other_id}'")

        if ffmpeg:
            started = ok_json(f"/api/livetv/channels/{lisek['id']}/play", "POST", {}, token)
            playlist_url = started["stream_url"]
            status, playlist, headers = request(playlist_url)
            assert status == 200 and b".ts" in playlist and "mpegurl" in headers["Content-Type"], playlist
            segment = next(line for line in playlist.decode().splitlines() if line.endswith(".ts"))
            status, chunk, headers = request(playlist_url.rsplit("/", 1)[0] + "/" + segment)
            assert status == 200 and headers["Content-Type"] == "video/mp2t" and chunk[:1] == b"\x47", (status, chunk[:4])
            wrong = playlist_url.replace(playlist_url.split("/")[-2], "0" * 32)
            assert request(wrong)[0] == 403
            assert request(playlist_url.replace("index.m3u8", "..%2F..%2Fetc%2Fpasswd"))[0] in (403, 404)
            # Someone else cannot stop it; the owner can.
            assert request(f"/api/livetv/sessions/{started['session_id']}", "DELETE", None, other_token)[0] == 404
            assert request(f"/api/livetv/sessions/{started['session_id']}", "DELETE", None, token)[0] == 204
            assert request(playlist_url)[0] == 403
            status, body, _ = request(f"/api/livetv/channels/{dead['id']}/play", "POST", {}, token)
            assert status == 502 and json.loads(body)["error_code"] == "live_stream_unavailable", (status, body)
        assert request(f"/api/admin/libraries/{library['id']}", "DELETE", None, token)[0] in (200, 204)
        assert ok_json("/api/livetv/status", token=token)["available"] is False
    finally:
        server.shutdown()
