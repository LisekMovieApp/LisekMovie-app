"""Central logs (Faza 17) for the disposable-database integration suite:
the day file under <config>/logs, a browser error reported through
`POST /api/logs/client` ([FRONTEND]), an unreadable video's ffprobe error
([FFPROBE]) and the admin endpoints that list and filter them."""

import json
import time
import urllib.parse


def check_logs(request, ok_json, token, other_token, root, has_ffprobe):
    logs_dir = root / "config" / "logs"
    files = sorted(p.name for p in logs_dir.iterdir())
    assert files and all(f.startswith("lisekmovie.") and f.endswith(".log") for f in files), files

    # A browser error, signed in or not.
    report = {"level": "error", "kind": "error", "message": "TypeError: x is undefined",
              "stack": "at load (app.js:1:2)", "url": "http://localhost/media/1", "source": "app.js:1:2"}
    assert request("/api/logs/client", "POST", report)[0] == 204
    assert request("/api/logs/client", "POST", dict(report, message="Warning from Lisek", level="warn"), other_token)[0] == 204
    assert request("/api/logs/client", "POST", dict(report, message="  "))[0] == 400

    if has_ffprobe:
        broken = root / "broken"
        broken.mkdir()
        (broken / "Zepsuty Film (2020).mkv").write_bytes(b"this is not a video")
        library = ok_json("/api/admin/libraries", "POST", {
            "name": "Logi smoke", "library_type": "movies", "root_paths": [str(broken)]}, token)
        status, body, _ = request(f"/api/admin/libraries/{library['id']}/scan", "POST", {}, token)
        assert status == 202, (status, body)
        job = json.loads(body)
        for _ in range(600):
            job = ok_json(f"/api/admin/scan-jobs/{job['id']}", token=token)
            if job["status"] in ("completed", "failed"):
                break
            time.sleep(.1)
        assert request(f"/api/admin/libraries/{library['id']}", "DELETE", None, token)[0] in (200, 204)

    listing = ok_json("/api/admin/logs", token=token)
    assert listing["retention_days"] == 30 and listing["files"], listing
    today = listing["files"][0]["date"]
    assert request("/api/admin/logs", token=other_token)[0] == 403
    assert request("/api/admin/logs/2026-13-45", token=token)[0] == 400
    assert request("/api/admin/logs/..%2Fsecret", token=token)[0] in (400, 404)
    assert request("/api/admin/logs/1999-01-01", token=token)[0] == 404

    # The file writer flushes in the background: give it a moment.
    for _ in range(50):
        frontend = ok_json(f"/api/admin/logs/{today}?" + urllib.parse.urlencode({"source": "frontend"}), token=token)
        if len(frontend["entries"]) >= 2:
            break
        time.sleep(.1)
    messages = [(e["level"], e["message"]) for e in frontend["entries"]]
    assert ("ERROR", "[FRONTEND] TypeError: x is undefined") in messages, messages
    assert ("WARN", "[FRONTEND] Warning from Lisek") in messages, messages
    error = next(e for e in frontend["entries"] if e["level"] == "ERROR")
    assert error["source"] == "frontend" and error["fields"]["stack"] == "at load (app.js:1:2)", error
    assert "user" not in error["fields"] or error["fields"]["user"] is None, "anonymous report"
    warning = next(e for e in frontend["entries"] if e["level"] == "WARN")
    assert warning["fields"]["user"], warning

    only_errors = ok_json(f"/api/admin/logs/{today}?" + urllib.parse.urlencode({"source": "frontend", "level": "error"}), token=token)
    assert {e["level"] for e in only_errors["entries"]} == {"ERROR"}, only_errors
    backend = ok_json(f"/api/admin/logs/{today}?source=backend&limit=5", token=token)
    assert 0 < len(backend["entries"]) <= 5 and backend["total"] >= len(backend["entries"]), backend
    assert all(e["source"] == "backend" for e in backend["entries"])
    if has_ffprobe:
        found = False
        for _ in range(50):
            probe = ok_json(f"/api/admin/logs/{today}?source=ffprobe", token=token)
            found = any(e["message"].startswith("[FFPROBE]") and "Zepsuty Film" in e["fields"].get("context", "")
                        for e in probe["entries"])
            if found:
                break
            time.sleep(.1)
        if not found:
            tail = ok_json(f"/api/admin/logs/{today}?limit=40", token=token)["entries"]
            raise AssertionError([(e["source"], e["level"], e["message"][:120]) for e in tail])

    status, body, headers = request(f"/api/admin/logs/{today}/raw", token=token)
    assert status == 200 and b"[FRONTEND] TypeError" in body, status
    assert "attachment" in headers["Content-Disposition"]
