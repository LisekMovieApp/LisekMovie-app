"""Watch statistics and hardware diagnostics (Faza 13) for the
disposable-database integration suite: the User-Agent of a playback start
lands in the session, the admin dashboard and session list, the bots'
summary (JSON and Discord payload) with an admin session or the read-only
stats key, and the hardware survey."""

import json
from urllib.parse import urlencode

TV_UA = ("Mozilla/5.0 (SMART-TV; LINUX; Tizen 7.0) AppleWebKit/537.36 "
         "(KHTML, like Gecko) 94.0.4606.31/7.0 TV Safari/537.36")


def check_stats(request, ok_json, sql, token, other_token, other_id, item_id, has_ffmpeg):
    # A playback started from a TV browser, watched for ten minutes, stopped at 40 %.
    status, body, _ = request(f"/api/media/{item_id}/playback", "POST", {}, token, {"User-Agent": TV_UA})
    assert status == 200, (status, body)
    sid = json.loads(body)["session_id"]
    row = sql(f"SELECT platform || '|' || device_type || '|' || left(user_agent, 20) FROM playback_sessions WHERE id = '{sid}'")
    assert row == "smart_tv|tv|Mozilla/5.0 (SMART-T", row
    ok_json("/api/telemetry/playback-progress", "POST", {
        "session_id": sid, "position_seconds": 40, "duration_seconds": 100, "paused": True, "event": "stop",
    }, token)
    sql(f"UPDATE playback_sessions SET watched_seconds = 600 WHERE id = '{sid}'")
    assert sql(f"SELECT duration_seconds::int || '|' || completed FROM playback_sessions WHERE id = '{sid}'") == "100|false"

    dash = ok_json("/api/admin/stats/dashboard?" + urlencode({"tz": "Europe/Warsaw", "platform": "smart_tv"}), token=token)
    assert dash["totals"]["sessions"] == 1 and dash["totals"]["plays"] == 1, dash["totals"]
    assert dash["totals"]["abandoned"] == 1 and dash["dropoff"][4] == 1, dash["dropoff"]
    assert dash["platforms"][0]["key"] == "smart_tv" and dash["days"], dash
    session = dash["sessions"][0]
    assert (session["id"], session["outcome"], session["position_seconds"]) == (sid, "abandoned", 40), session
    assert dash["top_movies"][0]["id"] == item_id, dash["top_movies"]
    everything = ok_json("/api/admin/stats/dashboard", token=token)
    assert everything["totals"]["watched_seconds"] >= 600
    assert ok_json("/api/admin/stats/sessions?offset=0", token=token)
    for bad in ("platform=tv", "content=episode", "tz=Mars/Base", "from=2026-01-02T00:00:00Z&to=2026-01-01T00:00:00Z"):
        assert request(f"/api/admin/stats/dashboard?{bad}", token=token)[0] == 400, bad
    assert request("/api/admin/stats/dashboard", token=other_token)[0] == 403

    # Bots: no credentials, then an admin session, then the read-only key.
    assert request("/api/stats/summary")[0] == 401
    summary = ok_json("/api/stats/summary?period=weekly", token=token)
    assert summary["period"] == "weekly" and " - " in summary["label"], summary
    assert summary["totals"]["active_users"] >= 1 and summary["top_movies"][0]["watched"], summary
    assert summary["top_movies"][0]["unique_users"] >= 1
    assert request("/api/stats/summary", token=other_token)[0] == 403, "a non-admin session is not enough"
    assert ok_json("/api/admin/stats/key", token=token)["configured"] is False
    key = ok_json("/api/admin/stats/key", "POST", {}, token=token)["key"]
    status, body, _ = request("/api/stats/summary?format=discord&period=daily", headers={"X-Lisek-Stats-Key": key})
    assert status == 200, (status, body)
    embeds = json.loads(body)["embeds"]
    assert embeds[0]["title"].startswith("📊 Statystyki oglądania") and "Łącznie:" in embeds[0]["footer"]["text"], embeds
    assert "👤)" in embeds[1]["description"], embeds
    assert request("/api/stats/summary", headers={"X-Lisek-Stats-Key": key + "x"})[0] == 401
    assert request("/api/stats/summary?period=yearly", headers={"X-Lisek-Stats-Key": key})[0] == 400
    assert request("/api/stats/summary?format=xml", headers={"X-Lisek-Stats-Key": key})[0] == 400
    assert request("/api/admin/stats/dashboard", headers={"X-Lisek-Stats-Key": key})[0] == 401, "the key reads the summary only"
    assert request("/api/admin/stats/key", "DELETE", None, token)[0] == 204
    assert request("/api/stats/summary", headers={"X-Lisek-Stats-Key": key})[0] == 401

    # Hardware survey: runs here without a GPU and still describes the machine.
    report = ok_json("/api/admin/hardware/diagnostics", "POST", {}, token=token)
    assert report["cpu"]["logical_cores"] >= 1 and report["os"], report
    if has_ffmpeg:
        assert report["ffmpeg"]["version"], report["ffmpeg"]
    assert isinstance(report["encoders"], list) and isinstance(report["warnings"], list)
    status = ok_json("/api/admin/hardware", token=token)
    assert status["report"]["generated_at"] == report["generated_at"] and "mode" in status["balancing"], status
    assert request("/api/admin/hardware/diagnostics", "POST", {}, other_token)[0] == 403
    assert isinstance(report["pool_candidates"], list)

    # Faza 14: the transcoding pool (an NVIDIA card with share 2 and an
    # Intel GPU through Quick Sync), saved from the panel and shown with
    # its load; then back to the previous mode.
    devices = [
        {"api": "nvenc", "device": "0", "name": "RTX", "enabled": True, "weight": 2, "max_sessions": 0,
         "decode_codecs": ["h264", "hevc"], "encode_codecs": ["h264", "hevc"]},
        {"api": "qsv", "device": "/dev/dri/renderD128", "name": "UHD 770", "enabled": True, "weight": 1,
         "max_sessions": 1, "decode_codecs": ["h264"], "encode_codecs": ["h264"]},
        {"api": "vaapi", "device": "/dev/dri/renderD129", "name": "Radeon", "enabled": False, "weight": 1,
         "max_sessions": 0, "decode_codecs": [], "encode_codecs": ["h264"]},
    ]
    saved = ok_json("/api/admin/hardware/pool", "PUT", {"devices": devices, "use_pool": True}, token)
    assert saved["balancing"]["mode"] == "pool", saved["balancing"]
    ids = [d["id"] for d in saved["balancing"]["devices"]]
    assert ids == ["nvenc:0", "qsv:/dev/dri/renderD128"], ids
    assert [d["weight"] for d in saved["balancing"]["devices"]] == [2, 1]
    assert ok_json("/api/admin/playback/settings", token=token)["hardware_acceleration"] == "pool"
    assert len(ok_json("/api/admin/hardware", token=token)["pool"]) == 3
    for bad in ({**devices[1], "device": "/etc/passwd"}, {**devices[1], "api": "amf"}, {**devices[1], "weight": 0}):
        assert request("/api/admin/hardware/pool", "PUT", {"devices": [bad], "use_pool": True}, token)[0] == 400, bad
    assert request("/api/admin/hardware/pool", "PUT", {"devices": [devices[0], devices[0]], "use_pool": True}, token)[0] == 400
    off = ok_json("/api/admin/hardware/pool", "PUT", {"devices": devices, "use_pool": False}, token)
    assert off["balancing"]["mode"] == "none" and off["balancing"]["devices"] == [], off["balancing"]
    assert request("/api/admin/hardware/pool", "PUT", {"devices": [], "use_pool": False}, other_token)[0] == 403
