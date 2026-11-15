"""Faza 26: own TV channels end to end on the disposable database — schedule,
planned timeline, a real shared HLS stream (files of different sizes, one
without audio), fillers, the IPTV export with API keys."""

import json
import shutil
import subprocess
import time
import urllib.error
import urllib.request

from phase20_smoke import scan
from profiles_smoke import png


def make_clip(ffmpeg, path, size, rate, seconds, audio):
    args = [ffmpeg, "-v", "error", "-f", "lavfi", "-i", f"testsrc2=size={size}:rate={rate}:duration={seconds}"]
    if audio:
        args += ["-f", "lavfi", "-i", f"sine=frequency={audio}:duration={seconds}", "-c:a", "aac"]
    args += ["-c:v", "libx264", "-pix_fmt", "yuv420p", str(path)]
    subprocess.run(args, check=True)


def get(base, path, headers=None):
    request = urllib.request.Request(base + path, headers=headers or {})
    try:
        response = urllib.request.urlopen(request, timeout=40)
    except urllib.error.HTTPError as error:
        response = error
    return response.status, response.read(), response.headers


def check_tv_channels(request, ok_json, sql, base, token, other_token, root, ffmpeg, ffprobe):
    films = root / "tv-films"
    films.mkdir()
    make_clip(ffmpeg, films / "Pierwszy (2001).mp4", "640x360", 25, 9, 440)
    make_clip(ffmpeg, films / "Drugi (2002).mp4", "320x240", 30, 9, None)   # no audio
    make_clip(ffmpeg, films / "Trzeci (2003).mp4", "1280x720", 24, 9, 880)
    library = ok_json("/api/admin/libraries", "POST", {"name": "Kanalowe", "library_type": "movies",
                      "root_paths": [str(films)], "save_metadata_in_media_folder": False}, token)
    fillers = root / "tv-fillers"
    fillers.mkdir()
    make_clip(ffmpeg, fillers / "reklama.mp4", "640x360", 25, 3, 300)
    try:
        scan(request, ok_json, token, library["id"])

        # Admin only.
        assert request("/api/admin/tv/channels", "GET", None, other_token)[0] == 403
        assert request("/api/tv/channels", "GET", None, None)[0] == 401

        # Channels: validation, numbering, uniqueness.
        body = {"name": "Lisek Kino", "timezone": "Europe/Warsaw", "quality": "480p"}
        for bad in ({**body, "name": " "}, {**body, "timezone": "Mars/Base"}, {**body, "quality": "4k"}, {**body, "number": "x"}):
            assert request("/api/admin/tv/channels", "POST", bad, token)[0] == 400, bad
        channel = ok_json("/api/admin/tv/channels", "POST", body, token)
        assert channel["number"] == "1" and channel["enabled"] and channel["quality"] == "480p", channel
        second = ok_json("/api/admin/tv/channels", "POST", {**body, "name": "Drugi kanał"}, token)
        assert second["number"] == "2", second
        status, answer, _ = request("/api/admin/tv/channels", "POST", {**body, "number": "2"}, token)
        assert status == 409 and json.loads(answer)["error_code"] == "number_taken", (status, answer)

        # Logo.
        upload = urllib.request.Request(base + f"/api/admin/tv/channels/{channel['id']}/logo", method="POST",
                                        data=png(300, 100, (200, 160, 77)), headers={"Authorization": f'LisekMovie Token="{token}"', "Content-Type": "image/png"})
        assert urllib.request.urlopen(upload, timeout=20).status == 204
        status, logo, headers = request(f"/api/tv/channels/{channel['id']}/logo", token=token)
        assert status == 200 and logo[:4] == b"\x89PNG", status
        bad_upload = urllib.request.Request(base + f"/api/admin/tv/channels/{channel['id']}/logo", method="POST",
                                            data=b"not an image", headers={"Authorization": f'LisekMovie Token="{token}"'})
        try:
            urllib.request.urlopen(bad_upload, timeout=20)
            raise AssertionError("a non-image logo was accepted")
        except urllib.error.HTTPError as error:
            assert error.code == 400

        # Search and the schedule.
        hits = ok_json("/api/admin/tv/sources?q=Kanalowe", token=token)
        source = next(h for h in hits if h["kind"] == "library")
        assert request("/api/admin/tv/sources?q=%25", "GET", None, token)[0] == 200
        slot = {"start_time": None, "start_mode": "strict", "days_of_week": 127, "source_kind": "library", "source_id": source["id"],
                "play_order": "chronological", "mode": "flood", "count": None, "duration_minutes": None, "tail_mode": "offline",
                "pre_filler_id": None, "mid_filler_id": None, "post_filler_id": None, "tail_filler_id": None,
                "title_override": None, "in_guide": True}
        assert request(f"/api/admin/tv/channels/{channel['id']}/slots", "PUT", [{**slot, "mode": "multiple", "count": None}], token)[0] == 400
        assert request(f"/api/admin/tv/channels/{channel['id']}/slots", "PUT", [{**slot, "days_of_week": 0}], token)[0] == 400
        assert request(f"/api/admin/tv/channels/{channel['id']}/slots", "PUT", [{**slot, "source_kind": "folder"}], token)[0] == 400

        # Fillers: a folder on the disk (absolute path only), scanned for durations.
        assert request("/api/admin/tv/fillers/folders", "POST", {"path": "relative/dir"}, token)[0] == 400
        folder = ok_json("/api/admin/tv/fillers/folders", "POST", {"path": str(fillers)}, token)
        assert request("/api/admin/tv/fillers/folders", "POST", {"path": str(fillers)}, token)[0] == 409
        for _ in range(100):
            files = ok_json(f"/api/admin/tv/fillers/folders/{folder['id']}/files", token=token)
            if files:
                break
            time.sleep(.2)
        assert [f["path"] for f in files] == [str(fillers / "reklama.mp4")] and 2500 < files[0]["duration_ms"] < 3500, files
        preset = ok_json("/api/admin/tv/fillers/presets", "POST", {"id": "00000000-0000-0000-0000-000000000000", "name": "Reklamy",
                          "kind": "post", "mode": "count", "count": 1, "duration_seconds": None, "pad_minutes": None,
                          "folder_ids": [folder["id"]]}, token)
        assert request("/api/admin/tv/fillers/presets", "POST", {**preset, "id": "00000000-0000-0000-0000-000000000000"}, token)[0] == 409

        slots = ok_json(f"/api/admin/tv/channels/{channel['id']}/slots", "PUT", [{**slot, "post_filler_id": preset["id"]}], token)
        assert len(slots) == 1 and slots[0]["source_label"], slots
        result = ok_json(f"/api/admin/tv/channels/{channel['id']}/rebuild", "POST", {"mode": "reset"}, token)
        assert result["planned"] > 10, result
        preview = ok_json(f"/api/admin/tv/channels/{channel['id']}/preview?hours=1", token=token)
        kinds = [p["kind"] for p in preview]
        assert kinds[0] == "content" and "post" in kinds, kinds
        titles = [p["title"] for p in preview if p["kind"] == "content"][:3]
        assert titles == ["Pierwszy", "Drugi", "Trzeci"], titles
        assert all(a["ends_at"] == b["starts_at"] for a, b in zip(preview, preview[1:])), "no gaps, no overlaps"

        # Viewers see it, with now and next.
        listed = ok_json("/api/tv/channels", token=other_token)
        mine = next(c for c in listed if c["id"] == channel["id"])
        assert mine["now"]["title"] == "Pierwszy" or mine["now"]["title"] in ("Drugi", "Trzeci"), mine
        assert mine["next"] is not None and mine["has_logo"], mine
        guide = ok_json("/api/tv/guide?hours=2", token=other_token)
        assert any(g["channel_id"] == channel["id"] and len(g["programmes"]) >= 2 for g in guide), guide

        # The stream: one shared HLS, files of different size and one without sound.
        started = ok_json(f"/api/tv/channels/{channel['id']}/play", "POST", None, other_token)
        url = started["stream_url"]
        status, playlist, headers = request(url)
        assert status == 200 and b".ts" in playlist and "mpegurl" in headers["Content-Type"], (status, playlist[:200])
        # Wait for a few segments so a seam between two files is covered.
        deadline = time.time() + 50
        names = []
        while time.time() < deadline:
            names = [l for l in request(url)[1].decode().splitlines() if l.endswith(".ts")]
            if len(names) >= 7:
                break
            time.sleep(1)
        assert len(names) >= 3, names
        directory = url.rsplit("/", 1)[0]
        status, segment, headers = request(f"{directory}/{names[-1]}")
        assert status == 200 and segment[:1] == b"\x47" and headers["Content-Type"] == "video/mp2t"
        if ffprobe:
            seg_path = root / "tv-seg.ts"
            seg_path.write_bytes(segment)
            streams = subprocess.run([ffprobe, "-v", "error", "-show_entries", "stream=codec_name,width,height", "-of", "csv=p=0", str(seg_path)],
                                     capture_output=True, text=True, check=True).stdout.split()
            assert "h264,854,480" in streams and any(s.startswith("aac") for s in streams), streams
            # Continuity across the seams between files (sizes, frame rates, a silent file):
            # no hole in the picture or the sound longer than half a second.
            joined = root / "tv-joined.ts"
            joined.write_bytes(b"".join(request(f"{directory}/{n}")[1] for n in names))
            for selector in ("v", "a"):
                out = subprocess.run([ffprobe, "-v", "error", "-select_streams", selector, "-show_entries", "packet=pts_time", "-of", "csv=p=0", str(joined)],
                                     capture_output=True, text=True, check=True).stdout
                times = sorted(float(x.strip(",")) for x in out.split() if x.strip(","))
                gaps = [b - a for a, b in zip(times, times[1:])]
                assert len(times) > 100 and max(gaps) < 0.5, (selector, len(times), max(gaps))
        assert request(url.replace(url.split("/")[-2], "0" * 32))[0] == 403
        # A second viewer shares the same stream.
        both = ok_json(f"/api/tv/channels/{channel['id']}/play", "POST", None, token)
        assert both["stream_url"] != url and request(both["stream_url"])[0] == 200
        admin_list = ok_json("/api/admin/tv/channels", token=token)
        running = next(c for c in admin_list if c["id"] == channel["id"])
        assert running["streaming"] and running["now_title"], running

        # IPTV export: API keys with a scope; the key in a header or in the query on /iptv only.
        stats_key = ok_json("/api/admin/api-keys", "POST", {"name": "Tylko statystyki"}, token)
        assert stats_key.get("scopes") == ["stats"], stats_key
        tv_key = ok_json("/api/admin/api-keys", "POST", {"name": "Kodi", "scopes": ["iptv"]}, token)
        assert request("/api/admin/api-keys", "POST", {"name": "Zły", "scopes": ["root"]}, token)[0] == 400
        assert get(base, "/iptv/channels.m3u")[0] == 401
        assert get(base, "/iptv/channels.m3u", {"X-Lisek-Api-Key": stats_key["key"]})[0] in (401, 403)
        assert get(base, f"/iptv/channels.m3u?key={stats_key['key']}")[0] in (401, 403)
        assert get(base, "/api/stats/summary", {"X-Lisek-Api-Key": tv_key["key"]})[0] in (401, 403), "iptv keys do not read statistics"
        assert get(base, f"/api/admin/api-keys?key={tv_key['key']}")[0] == 401, "the query key works on /iptv only"
        assert get(base, f"/api/tv/channels?key={tv_key['key']}")[0] == 401
        status, m3u, headers = get(base, "/iptv/channels.m3u", {"X-Lisek-Api-Key": tv_key["key"]})
        text = m3u.decode()
        assert status == 200 and text.startswith("#EXTM3U") and f'tvg-id="{channel["id"]}"' in text and "key=" not in text, text
        status, m3u, _ = get(base, f"/iptv/channels.m3u?key={tv_key['key']}")
        text = m3u.decode()
        assert f"/iptv/stream/{channel['id']}/index.m3u8?key={tv_key['key']}" in text and 'tvg-name="Lisek Kino"' in text, text
        assert 'tvg-chno="1"' in text and "tvg-logo=" in text
        status, epg, _ = get(base, f"/iptv/epg.xml?key={tv_key['key']}")
        assert status == 200 and f'<channel id="{channel["id"]}">' in epg.decode() and "<programme " in epg.decode()
        status, playlist, _ = get(base, f"/iptv/stream/{channel['id']}/index.m3u8?key={tv_key['key']}")
        assert status == 200 and f"?key={tv_key['key']}" in playlist.decode(), playlist
        seg = next(l for l in playlist.decode().splitlines() if ".ts" in l).split("?")[0]
        assert get(base, f"/iptv/stream/{channel['id']}/{seg}")[0] == 401
        status, chunk, _ = get(base, f"/iptv/stream/{channel['id']}/{seg}", {"X-Lisek-Api-Key": tv_key["key"]})
        assert status == 200 and chunk[:1] == b"\x47", status
        assert get(base, f"/iptv/stream/{channel['id']}/..%2Fx.ts?key={tv_key['key']}")[0] == 404

        # Disabling a channel stops it and takes it out of the lists.
        body = {**body, "enabled": False}
        assert request(f"/api/admin/tv/channels/{channel['id']}", "PUT", {**body, "number": "1"}, token)[0] == 200
        assert request(url)[0] == 403
        assert all(c["id"] != channel["id"] for c in ok_json("/api/tv/channels", token=other_token))
        assert f'tvg-id="{channel["id"]}"' not in get(base, f"/iptv/channels.m3u?key={tv_key['key']}")[1].decode()

        # A channel with nothing planned still streams (the slate fills the gap).
        empty = ok_json(f"/api/tv/channels/{second['id']}/play", "POST", None, other_token)
        status, playlist, _ = request(empty["stream_url"])
        assert status == 200 and b".ts" in playlist, (status, playlist[:200])

        # Copying a schedule, then deleting.
        copied = ok_json(f"/api/admin/tv/channels/{second['id']}/copy-schedule", "POST", {"from_channel_id": channel["id"]}, token)
        assert len(copied) == 1 and copied[0]["source_id"] == source["id"]
        assert request(f"/api/admin/tv/channels/{second['id']}/copy-schedule", "POST", {"from_channel_id": second["id"]}, token)[0] == 404
        for cid in (channel["id"], second["id"]):
            assert request(f"/api/admin/tv/channels/{cid}", "DELETE", None, token)[0] == 204
        assert sql("SELECT count(*) FROM tv_playout_items") == "0" and sql("SELECT count(*) FROM tv_slots") == "0"
        ok_json("/api/admin/tv/fillers/presets/" + preset["id"], "DELETE", None, token)
        ok_json("/api/admin/tv/fillers/folders/" + folder["id"], "DELETE", None, token)
        for key in (stats_key, tv_key):
            ok_json(f"/api/admin/api-keys/{key['id']}", "DELETE", None, token)
    finally:
        request(f"/api/admin/libraries/{library['id']}", "DELETE", None, token)
