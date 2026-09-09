"""Run the real server against a disposable PostgreSQL database.

Usage: python tools/smoke_test.py --server PATH --psql PATH --database-url URL
The supplied PostgreSQL role must be allowed to create databases.
"""
import argparse
import contextlib
import concurrent.futures
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--server", required=True)
    parser.add_argument("--psql", default="psql")
    parser.add_argument("--database-url", required=True)
    parser.add_argument("--ffmpeg")
    parser.add_argument("--ffprobe")
    args = parser.parse_args()
    database = "lisek_smoke_" + uuid.uuid4().hex
    admin_url = urllib.parse.urlsplit(args.database_url)
    test_url = urllib.parse.urlunsplit(admin_url._replace(path="/" + database))
    env = os.environ.copy()
    env.pop("DATABASE_URL", None)
    env["LISEK_ALLOW_LOCAL_WEBHOOKS"]="1"
    if args.ffmpeg: env["LISEK_FFMPEG"]=str(Path(args.ffmpeg).resolve())
    if args.ffprobe: env["LISEK_FFPROBE"]=str(Path(args.ffprobe).resolve())
    server_path = str(Path(args.server).resolve())
    def sql(statement, url=test_url):
        result = subprocess.run([args.psql, url, "-XqAt", "-v", "ON_ERROR_STOP=1", "-c", statement],
                                capture_output=True, text=True, check=True)
        return result.stdout.strip()
    sql('CREATE DATABASE ' + database, args.database_url)
    server = None
    try:
        with tempfile.TemporaryDirectory(prefix="lisek-smoke-") as directory, contextlib.ExitStack() as cleanup:
            root = Path(directory)
            media = root / "media"
            media.mkdir()
            movie_path=media / "Test Movie (2024).mp4"
            content = bytes(range(256)) * 100
            if args.ffmpeg:
                subprocess.run([args.ffmpeg,"-v","error","-f","lavfi","-i","testsrc=size=320x180:rate=24","-t","8","-c:v","libx264","-pix_fmt","yuv420p",str(movie_path)],check=True)
                content=movie_path.read_bytes()
            else:
                movie_path.write_bytes(content)
            class WebhookTarget(BaseHTTPRequestHandler):
                attempts=0
                def do_POST(self):
                    self.rfile.read(int(self.headers.get('Content-Length','0')))
                    WebhookTarget.attempts+=1
                    self.send_response(503 if WebhookTarget.attempts==1 else 200)
                    self.end_headers()
                def log_message(self,*args): pass
            target=ThreadingHTTPServer(('127.0.0.1',0),WebhookTarget)
            threading.Thread(target=target.serve_forever,daemon=True).start()
            cleanup.callback(target.server_close)
            cleanup.callback(target.shutdown)
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            env["LISEK_BIND_ADDR"] = f"127.0.0.1:{port}"
            env["LISEK_CONFIG_DIR"] = str(root / "config")
            base = f"http://127.0.0.1:{port}"
            log = open(root / "server.log", "w", encoding="utf-8")
            cleanup.callback(log.close)
            server = subprocess.Popen([server_path], cwd=root, env=env, stdout=log, stderr=log)
            def stop_server(process):
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)
            cleanup.callback(stop_server, server)
            def request(path, method="GET", data=None, token=None, headers=None):
                request_headers = dict(headers or {})
                if token:
                    request_headers["Authorization"] = f'LisekMovie Token="{token}"'
                if data is not None:
                    request_headers["Content-Type"] = "application/json"
                req = urllib.request.Request(base + path, method=method, headers=request_headers,
                    data=None if data is None else json.dumps(data).encode())
                try:
                    response = urllib.request.urlopen(req, timeout=30)
                except urllib.error.HTTPError as error:
                    response = error
                body = response.read()
                return response.status, body, response.headers
            def ok_json(path, method="GET", data=None, token=None):
                status, body, _ = request(path, method, data, token)
                assert status in (200, 201, 204), (path, status, body)
                return json.loads(body) if body else None
            for attempt in range(100):
                try:
                    if request("/api/health")[0] == 200:
                        break
                except OSError:
                    pass
                time.sleep(.1)
            else:
                raise AssertionError("Server did not start: " + (root / "server.log").read_text())
            assert ok_json("/api/status")["setup_required"]
            config = {
                "database": {"host": admin_url.hostname, "port": admin_url.port or 5432,
                    "database": database, "username": urllib.parse.unquote(admin_url.username or ""),
                    "password": urllib.parse.unquote(admin_url.password or "")},
                "admin_username": "test-admin", "admin_password": uuid.uuid4().hex,
                "ui_language": "en"
            }
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
                results = list(executor.map(lambda _: request("/api/setup/finish", "POST", config)[0], range(2)))
            assert sorted(results) == [200, 400], results
            assert not ok_json("/api/status")["setup_required"]
            assert sql("SELECT count(*) FROM users WHERE is_admin") == "1"
            assert request("/api/setup/database/test", "POST", config["database"])[0] == 400
            login = ok_json("/api/auth/login", "POST", {"username": config["admin_username"],
                "password": config["admin_password"], "client": "smoke", "device": "test", "device_id": "smoke"})
            token = login["token"]
            assert request("/api/libraries")[0] == 401
            assert request("/api/libraries?token=" + token)[0] == 401
            spec = ok_json("/api/openapi.json")
            assert "/api/auth/login" in spec["paths"]
            assert "/api/media/{item_id}/playback" in spec["paths"]
            sql("UPDATE provider_config SET enabled=false")
            ok_json('/api/admin/webhooks','POST',{'name':'Local test','url':f'http://127.0.0.1:{target.server_port}/notify','destination_type':'generic_json','enabled':True,'subscribed_events':['item_added']},token)
            library = ok_json("/api/admin/libraries", "POST", {"name":"Smoke","library_type":"movies",
                "root_paths":[str(media)],"save_metadata_in_media_folder":False},token)
            library_id = library["id"]
            ok_json(f"/api/admin/libraries/{library_id}/scan", "POST", {}, token)
            item_id = sql("SELECT id FROM media_items LIMIT 1")
            assert item_id
            session = ok_json(f"/api/media/{item_id}/playback", "POST", {}, token)
            sid = session["session_id"]
            status, body, headers = request(session["stream_url"],headers={"Range":"bytes=10-19"})
            assert status == 206 and body == content[10:20]
            assert headers["Content-Range"] == f"bytes 10-19/{len(content)}"
            status, body, _ = request(session["stream_url"],headers={"Range":"bytes=-16"})
            assert status == 206 and body == content[-16:]
            assert request(session["stream_url"],headers={"Range":"bytes=999999-"} )[0] == 416
            assert request(f"/api/playback/{sid}/stream?ticket=invalid")[0] == 401
            progress={"position_seconds":25,"duration_seconds":100,"paused":False}
            ok_json(f"/api/playback/{sid}/progress","PUT",progress,token)
            sql(f"UPDATE playback_sessions SET last_heartbeat_at=now()-interval '25 seconds' WHERE id='{sid}'")
            progress["position_seconds"]=50
            progress["paused"]=True
            ok_json(f"/api/playback/{sid}/progress","PUT",progress,token)
            viewed=float(sql(f"SELECT watched_seconds FROM playback_sessions WHERE id='{sid}'"))
            assert 24 <= viewed <= 30
            sql(f"UPDATE playback_sessions SET last_heartbeat_at=now()-interval '25 seconds' WHERE id='{sid}'")
            ok_json(f"/api/playback/{sid}/progress","PUT",progress,token)
            assert abs(float(sql(f"SELECT watched_seconds FROM playback_sessions WHERE id='{sid}'"))-viewed)<.001
            resumed = ok_json(f"/api/media/{item_id}/playback","POST",{},token)
            assert resumed["position_seconds"] == 50
            ok_json(f"/api/media/{item_id}/state","PUT",{"favorite":True},token)
            assert ok_json("/api/me/list",token=token)[0]["favorite"]
            assert len(ok_json("/api/admin/stats/users",token=token)) == 1
            assert ok_json("/api/games",token=token) == []
            assert request("/api/games/flappy-bird/scores","POST",{"score":10},token)[0] == 404
            ok_json("/api/admin/games/flappy-bird","PUT",{"enabled":True},token)
            assert len(ok_json("/api/games",token=token)) == 1
            ok_json("/api/games/flappy-bird/scores","POST",{"score":10},token)
            assert ok_json("/api/games/flappy-bird/leaderboard",token=token)[0]["score"] == 10
            ok_json("/api/admin/games/flappy-bird","PUT",{"enabled":False},token)
            assert request("/api/games/flappy-bird/leaderboard",token=token)[0] == 404
            if args.ffmpeg and args.ffprobe:
                sql(f"UPDATE user_media_state SET position_seconds=0,watched=false WHERE item_id='{item_id}'")
                converted=ok_json(f'/api/media/{item_id}/playback?transcode=true','POST',{},token)
                status,manifest,_=request(converted['stream_url'])
                assert status==200 and b'#EXTM3U' in manifest,(status,manifest)
                segment=next(line for line in manifest.decode().splitlines() if line and not line.startswith('#'))
                segment_url=f"/api/playback/{converted['session_id']}/hls/"+segment
                status,segment_body,_=request(segment_url)
                assert status==200 and len(segment_body)>188
                ok_json(f"/api/playback/{converted['session_id']}/progress",'PUT',{'position_seconds':1,'duration_seconds':8,'paused':True,'ended':True},token)
                assert request(segment_url)[0]==401
                ok_json('/api/admin/playback/safety','PUT',{'enabled':True,'max_width':100,'max_height':100,'max_bitrate_kbps':None},token)
                assert request(f'/api/media/{item_id}/playback?transcode=true','POST',{},token)[0]==403
                assert sql('SELECT count(*) FROM transcode_kills')=='1'
            for _ in range(100):
                if WebhookTarget.attempts>=2: break
                time.sleep(.1)
            assert WebhookTarget.attempts==2,WebhookTarget.attempts
            for _ in range(30):
                if sql("SELECT count(*) FROM webhook_deliveries WHERE status='delivered'")=='1':break
                time.sleep(.1)
            assert sql("SELECT count(*) FROM webhook_delivery_attempts")=='2'
            assert sql("SELECT status FROM webhook_deliveries LIMIT 1")=='delivered'
            # A second account cannot mutate another account's playback or reach admin APIs.
            other_id=sql("INSERT INTO users(username,password_hash) VALUES ('other','unused') RETURNING id")
            import hashlib,base64
            other_token=uuid.uuid4().hex
            other_hash=base64.urlsafe_b64encode(hashlib.sha256(other_token.encode()).digest()).decode().rstrip("=")
            sql(f"INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ('{other_id}','{other_hash}',now()+interval '1 hour')")
            assert request(f"/api/playback/{sid}/progress","PUT",progress,other_token)[0] == 404
            assert request("/api/admin/stats/users",token=other_token)[0] == 403
            assert ok_json("/api/me/list",token=other_token) == []
            ok_json("/api/auth/logout","POST",{},token)
            assert request(resumed["stream_url"])[0] == 401
            assert request("/api/auth/me",token=token)[0] == 401
            server.terminate()
            server.wait(timeout=10)
            log.close()
            server = None
            print("PASS: migrations, concurrent setup, auth, scan, ranges, resume, ownership, stats, games, ticket revocation, webhook retry, optional FFmpeg/HLS")
    finally:
        if server:
            server.terminate()
            server.wait(timeout=10)
        sql("DROP DATABASE " + database + " WITH (FORCE)",args.database_url)

if __name__ == "__main__":
    main()

