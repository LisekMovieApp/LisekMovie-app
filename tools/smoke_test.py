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
import queue
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SRT = ("1\n00:00:01,000 --> 00:00:03,000\nPierwszy napis\n\n"
       "2\n00:00:21,500 --> 00:00:23,000\nZażółć gęślą jaźń\n\n"
       "3\n00:00:42,000 --> 00:00:45,000\nPo przewinięciu\n")
CHROME = {"containers":["mp4"],"video_codecs":["h264","vp9","av1"],"audio_codecs":["aac","mp3","opus","flac"],
          "max_audio_channels":2,"hls":"mse"}

def scan_library(request,ok_json,token,name,folder):
    library=ok_json("/api/admin/libraries","POST",{"name":name,"library_type":"movies","root_paths":[str(folder)],
        "save_metadata_in_media_folder":False},token)
    status,body,_=request(f"/api/admin/libraries/{library['id']}/scan","POST",{},token)
    assert status==202,(status,body)
    job=json.loads(body)
    for _ in range(600):
        job=ok_json(f"/api/admin/scan-jobs/{job['id']}",token=token)
        if job["status"] in ("completed","failed"): break
        time.sleep(.1)
    assert job["status"]=="completed" and job["movies_found"]>=1,job
    return library["id"]

def check_hls_engine(args,root,request,ok_json,sql,token,base):
    """Remux, subtitles, seeking and burn-in through the real server."""
    # Earlier checks leave a 100x100 transcode limit behind.
    ok_json('/api/admin/playback/safety','PUT',{'enabled':False,'max_width':1920,'max_height':1080,'max_bitrate_kbps':None},token)
    folder=root/"hls"; folder.mkdir()
    srt=root/"embedded.srt"; srt.write_text(SRT,encoding="utf-8")
    film=folder/"Lisek Story (2020).mkv"
    subprocess.run([args.ffmpeg,"-v","error","-f","lavfi","-i","testsrc2=size=640x360:rate=24000/1001:duration=60",
        "-f","lavfi","-i","sine=frequency=440:sample_rate=48000:duration=60","-i",str(srt),
        "-filter_complex","[1:a]pan=5.1|FL=c0|FR=c0|FC=c0|LFE=c0|BL=c0|BR=c0[a]","-map","0:v","-map","[a]","-map","2:s",
        "-c:v","libx264","-preset","ultrafast","-bf","3","-g","48","-c:a","ac3","-b:a","448k","-c:s","srt",
        "-metadata:s:s:0","language=pol",str(film)],check=True)
    (folder/"Lisek Story (2020).pol.srt").write_bytes(SRT.encode("cp1250"))
    library_id=scan_library(request,ok_json,token,"HLS",folder)
    item=sql(f"SELECT id FROM media_items WHERE library_id='{library_id}'")

    # Chrome: MKV and AC-3 unsupported -> video copied, audio to stereo AAC.
    started=ok_json(f"/api/media/{item}/playback","POST",{"client":CHROME},token)
    assert started["mode"]=="remux" and started["hls"],started
    assert {"container_not_supported","audio_codec_not_supported"}<={*started["reasons"]},started["reasons"]
    delivered=started["delivered"]
    assert delivered["video_copied"] and delivered["audio_codec"]=="aac" and delivered["downmix"]=="standard",delivered
    assert [s["id"] for s in started["subtitle_tracks"]]==["s2","x0"],started["subtitle_tracks"]
    assert [s["hls_rendition"] for s in started["subtitle_tracks"]]==[0,1]
    sid=started["session_id"]; ticket=started["stream_url"].split("ticket=")[1]
    status,master,_=request(started["stream_url"])
    master=master.decode()
    assert status==200 and 'CODECS="avc1.' in master and 'LANGUAGE="pol"' in master and 'SUBTITLES="subs"' in master,master
    # FFmpeg as an HLS client: every playlist, the init segment and all media segments.
    out=subprocess.run([args.ffmpeg,"-v","error","-i",base+started["stream_url"],"-map","0:v","-map","0:a","-f","null","-"],
        capture_output=True,text=True,timeout=300)
    assert out.returncode==0,out.stderr
    status,cue,_=request(f"/api/playback/{sid}/hls/subs-s2-0.vtt?ticket={ticket}")
    assert status==200 and "Pierwszy napis" in cue.decode() and "X-TIMESTAMP-MAP" in cue.decode(),cue
    status,cue,_=request(f"/api/playback/{sid}/hls/subs-x0-0.vtt?ticket={ticket}")
    assert status==200 and "Zażółć gęślą jaźń" in cue.decode(),cue
    # Far seek: the server restarts FFmpeg there.
    status,playlist,_=request(f"/api/playback/{sid}/hls/video.m3u8?ticket={ticket}")
    last=[l for l in playlist.decode().splitlines() if l.startswith("seg-")][-1]
    started_at=time.time()
    status,body,_=request(f"/api/playback/{sid}/hls/{last}")
    assert status==200 and body[4:8]==b"moof",(status,body[:16])
    seek_seconds=time.time()-started_at
    ok_json(f"/api/playback/{sid}","DELETE",token=token)
    assert request(f"/api/playback/{sid}/hls/seg-0.m4s?ticket={ticket}")[0]==401

    # A TV plays MKV + AC-3 directly; the external subtitle becomes a <track> file.
    tv=dict(CHROME,containers=["mkv","mp4"],audio_codecs=["aac","ac3"],max_audio_channels=6,hls="native")
    direct=ok_json(f"/api/media/{item}/playback","POST",{"client":tv},token)
    assert direct["mode"]=="direct_play" and not direct["reasons"],direct
    track=next(s for s in direct["subtitle_tracks"] if s["id"]=="x0")
    status,vtt,headers=request(track["url"])
    assert status==200 and headers["Content-Type"].startswith("text/vtt") and "Zażółć gęślą jaźń" in vtt.decode(),vtt
    ok_json(f"/api/playback/{direct['session_id']}","DELETE",token=token)

    # Burnt-in CP1250 subtitle + night-mode downmix: a transcode starting at 20 s.
    burned=ok_json(f"/api/media/{item}/playback","POST",{"client":CHROME,"subtitle":"x0","burn_subtitle":True,
        "downmix":"night","start_position_seconds":20},token)
    assert burned["mode"]=="transcode" and burned["subtitle_delivery"]=="burn_in",burned
    assert "subtitle_burn_in" in burned["reasons"] and burned["delivered"]["downmix"]=="night",burned
    status,playlist,_=request(burned["stream_url"].replace("master.m3u8","video.m3u8"))
    segment=[l for l in playlist.decode().splitlines() if l.startswith("seg-")][5]
    status,body,_=request(f"/api/playback/{burned['session_id']}/hls/{segment}")
    assert status==200 and body[4:8]==b"moof",(status,body[:16])
    ok_json(f"/api/playback/{burned['session_id']}","DELETE",token=token)

    if args.node and args.hlsjs:
        # VP9 + Opus decode in Playwright's Chromium (H.264 does not): remux and play with hls.js.
        web_folder=root/"web"; web_folder.mkdir()
        web=web_folder/"Another Story (2021).mkv"
        subprocess.run([args.ffmpeg,"-v","error","-f","lavfi","-i","testsrc2=size=320x180:rate=24:duration=60",
            "-f","lavfi","-i","sine=frequency=440:sample_rate=48000:duration=60","-i",str(srt),"-map","0:v","-map","1:a","-map","2:s",
            "-c:v","libvpx-vp9","-deadline","realtime","-cpu-used","8","-g","48","-b:v","300k","-c:a","libopus","-c:s","srt",
            str(web)],check=True)
        scan_library(request,ok_json,token,"Web",web_folder)
        web_item=sql("SELECT id FROM media_items WHERE file_path LIKE '%Another Story%'")
        played=ok_json(f"/api/media/{web_item}/playback","POST",{"client":CHROME},token)
        assert played["mode"]=="remux",played
        out=subprocess.run([args.node,str(Path(__file__).with_name("hls_browser_check.mjs").resolve()),
            json.dumps({"base":base,"master":played["stream_url"],"hlsjs":args.hlsjs,"seek":40})],
            capture_output=True,text=True,timeout=240)
        result=json.loads(out.stdout.strip().splitlines()[-1]) if out.stdout.strip() else {"ok":False,"error":out.stderr}
        assert result["ok"] and not result["errors"],result
        assert abs(result["duration"]-60)<1 and "Po przewinięciu" in " ".join(result["cues"]),result
        print(f"hls.js: duration {result['duration']:.1f}s, seek to 40s playing after {result['seekMs']:.0f} ms")
        ok_json(f"/api/playback/{played['session_id']}","DELETE",token=token)
    print(f"HLS engine: remux + subtitles OK, far seek served in {seek_seconds:.2f}s")

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--server", required=True)
    parser.add_argument("--psql", default="psql")
    parser.add_argument("--database-url", required=True)
    parser.add_argument("--ffmpeg")
    parser.add_argument("--ffprobe")
    parser.add_argument("--node")
    parser.add_argument("--hlsjs", help="hls.min.js: also play HLS in headless Chromium (needs playwright)")
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
                subprocess.run([args.ffmpeg,"-v","error","-f","lavfi","-i","testsrc2=size=640x360:rate=24","-t","8","-c:v","libx264","-pix_fmt","yuv420p",str(movie_path)],check=True)
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
                assert status in (200, 201, 202, 204), (path, status, body)
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
            assert "post" in spec["paths"]["/api/admin/libraries"] and "put" in spec["paths"]["/api/admin/media/{item_id}"]
            # Provider API keys are stored sealed, never as plaintext.
            ok_json('/api/admin/providers/tmdb/key','PUT',{'api_key':'smoke-secret-key'},token)
            stored=sql("SELECT settings::text FROM provider_config WHERE provider_key='tmdb'")
            assert 'smoke-secret-key' not in stored and '"api_key_sealed": "v1:' in stored, stored
            assert any(p['provider_key']=='tmdb' and p['is_configured'] for p in ok_json('/api/admin/providers',token=token))
            ok_json('/api/admin/providers/tmdb/key','DELETE',token=token)
            assert sql("SELECT settings::text FROM provider_config WHERE provider_key='tmdb'") == '{}'
            # CORS is closed unless LISEK_CORS_ORIGINS lists an origin.
            _,_,cors_headers=request('/api/health',headers={'Origin':'http://evil.example'})
            assert 'Access-Control-Allow-Origin' not in cors_headers
            settings=ok_json('/api/admin/playback/settings',token=token)
            assert settings['min_resume_percentage']==5 and settings['max_resume_percentage']==90 and settings['min_resume_duration_seconds']==300
            invalid=dict(settings,min_resume_percentage=95)
            assert request('/api/admin/playback/settings','PUT',invalid,token)[0]==400
            settings['min_resume_duration_seconds']=0
            settings['encoding_threads']=2
            ok_json('/api/admin/playback/settings','PUT',settings,token)
            assert ok_json('/api/admin/playback/settings',token=token)==settings
            if args.ffmpeg:
                caps=ok_json('/api/admin/playback/capabilities','POST',settings,token)
                assert caps['ffmpeg_path']==str(Path(args.ffmpeg).resolve())
                assert 'ffmpeg version' in caps['version']
            notification_queue=queue.Queue()
            if args.node:
                probe=subprocess.Popen([args.node,str(Path(__file__).with_name('notification_probe.mjs').resolve())],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
                cleanup.callback(stop_server,probe)
                probe.stdin.write(json.dumps({'base':base,'token':token})+'\n');probe.stdin.flush()
                def collect():
                    for line in probe.stdout: notification_queue.put(json.loads(line))
                threading.Thread(target=collect,daemon=True).start()
                assert notification_queue.get(timeout=15)['type']=='ready'
            sql("UPDATE provider_config SET enabled=false")
            ok_json('/api/admin/webhooks','POST',{'name':'Local test','url':f'http://127.0.0.1:{target.server_port}/notify','destination_type':'generic_json','enabled':True,'subscribed_events':['item_added']},token)
            library = ok_json("/api/admin/libraries", "POST", {"name":"Smoke","library_type":"movies",
                "root_paths":[str(media)],"save_metadata_in_media_folder":False},token)
            library_id = library["id"]
            # Scans are queued (202) and run by the background worker.
            status, body, _ = request(f"/api/admin/libraries/{library_id}/scan", "POST", {}, token)
            assert status == 202, (status, body)
            job = json.loads(body)
            for _ in range(300):
                job = ok_json(f"/api/admin/scan-jobs/{job['id']}", token=token)
                if job["status"] in ("completed", "failed"):
                    break
                time.sleep(.1)
            assert job["status"] == "completed" and job["phase"] == "done", job
            assert (job["files_found"], job["movies_found"], job["unrecognized"]) == (1, 1, 0), job
            assert any(j["id"] == job["id"] for j in ok_json("/api/admin/scan-jobs", token=token))
            item_id = sql("SELECT id FROM media_items LIMIT 1")
            assert item_id
            if args.ffmpeg and args.ffprobe:
                # Streams are probed once, during the scan.
                assert job["probed"] == 1, job
                assert sql(f"SELECT count(*) FROM media_streams WHERE media_item_id='{item_id}' AND stream_type='video'") == "1"
                assert sql(f"SELECT probed_at IS NOT NULL AND file_size IS NOT NULL FROM media_items WHERE id='{item_id}'") == "t"
                # Faza 5: trickplay sheets are made in the background after the scan.
                for _ in range(300):
                    status, body, _ = request(f"/api/media/{item_id}/trickplay", token=token)
                    if status == 200:
                        break
                    assert status == 404 and b"trickplay_not_ready" in body, (status, body)
                    time.sleep(.2)
                assert status == 200, "no trickplay sheets: " + (root / "server.log").read_text()[-3000:]
                trick = json.loads(body)
                assert (trick["interval_seconds"], trick["width"], trick["height"], trick["columns"]) == (2, 320, 180, 10), trick
                assert trick["count"] == 4 and len(trick["sheets"]) == 1, trick
                status, sheet, headers = request(trick["sheets"][0] + "&token=" + token)
                assert status == 200 and sheet[:2] == b"\xff\xd8" and headers["Content-Type"] == "image/jpeg", (status, sheet[:4])
                assert "immutable" in headers["Cache-Control"]
                assert request(trick["sheets"][0])[0] == 401
                assert request(f"/api/media/{item_id}/trickplay?token=" + token)[0] == 401
                assert ok_json("/api/admin/trickplay", token=token)["ready"] == 1
                assert request(f"/api/admin/media/{item_id}/trickplay", "POST", None, token)[0] == 202
            if args.node:
                progress_seen = False
                while True:
                    event=notification_queue.get(timeout=15)
                    if event['type']=='scan_progress':
                        progress_seen |= event['job']['id']==job['id']
                        continue
                    break
                assert progress_seen, "admin sockets receive scan_progress"
                assert event['type']=='library_changed' and event['items'][0]['item_id']==item_id,event
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
            # Resume policy boundaries are applied by the real persistence path.
            for position,expected,watched in [(4,0,False),(5,5,False),(90,90,False),(91,0,True)]:
                ok_json(f"/api/media/{item_id}/state","PUT",{"watched":False},token)
                ok_json(f"/api/playback/{sid}/progress","PUT",dict(progress,position_seconds=position),token)
                state=ok_json(f"/api/media/{item_id}/state",token=token)
                assert state['position_seconds']==expected and state['watched']==watched,state
            ok_json(f"/api/media/{item_id}/state","PUT",{"watched":False},token)
            ok_json(f"/api/playback/{sid}/progress","PUT",progress,token)
            resumed = ok_json(f"/api/media/{item_id}/playback","POST",{},token)
            assert resumed["position_seconds"] == 50
            assert request(f"/api/media/{item_id}/next", token=token)[0] == 204, "a movie has no next episode"
            # Telemetry (Faza 4): the player's report saves the resume point and quality counters.
            telemetry={"session_id":resumed["session_id"],"position_seconds":45,"duration_seconds":100,"paused":False,
                "event":"heartbeat","stalls":2,"stall_seconds":1.5,"dropped_frames":3,"bandwidth_kbps":12000}
            ack=ok_json("/api/telemetry/playback-progress","POST",telemetry,token)
            assert ack["resume_position_seconds"]==45 and ack["next_report_seconds"]==15 and not ack["watched"],ack
            quality=[q for q in ok_json("/api/admin/stats/playback?limit=5",token=token) if q["id"]==resumed["session_id"]]
            assert quality and quality[0]["stall_count"]==2 and quality[0]["bandwidth_kbps"]==12000,quality
            assert request("/api/telemetry/playback-progress","POST",dict(telemetry,position_seconds=-1),token)[0]==400
            assert ok_json(f"/api/media/{item_id}/state",token=token)["position_seconds"]==45
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
                hls_base=f"/api/playback/{converted['session_id']}/hls/"
                variant=next(line for line in manifest.decode().splitlines() if line and not line.startswith('#'))
                status,media_playlist,_=request(hls_base+variant)
                assert status==200 and b'#EXT-X-PLAYLIST-TYPE:VOD' in media_playlist and b'#EXT-X-ENDLIST' in media_playlist,media_playlist
                segment=next(line for line in media_playlist.decode().splitlines() if line.startswith('seg-'))
                segment_url=hls_base+segment
                status,segment_body,headers=request(segment_url)
                assert status==200 and len(segment_body)>1000 and segment_body[4:8]==b'moof',segment_body[:16]
                ok_json(f"/api/playback/{converted['session_id']}/progress",'PUT',{'position_seconds':1,'duration_seconds':8,'paused':True,'ended':True},token)
                assert request(segment_url)[0]==401
                # Trusted reverse proxy detects the remote address; direct URLs cannot bypass the cap.
                settings['streaming_bitrate_limit_mbps']=0.25
                settings['trusted_proxies']=['127.0.0.1/32']
                ok_json('/api/admin/playback/settings','PUT',settings,token)
                remote_headers={'X-Forwarded-For':'8.8.8.8'}
                assert request(resumed['stream_url'],headers=remote_headers)[0]==403
                status,body,_=request(f'/api/media/{item_id}/playback','POST',{},token,remote_headers)
                assert status==200,(status,body)
                remote=json.loads(body)
                assert remote['mode']=='transcode',remote
                assert request(remote['stream_url'],headers=remote_headers)[0]==200
                ok_json(f"/api/playback/{remote['session_id']}/progress",'PUT',{'position_seconds':0,'duration_seconds':8,'paused':True,'ended':True},token)
                settings['streaming_bitrate_limit_mbps']=None
                ok_json('/api/admin/playback/settings','PUT',settings,token)
                if caps['nvenc_available']:
                    settings['hardware_acceleration']='nvenc'
                    ok_json('/api/admin/playback/settings','PUT',settings,token)
                    hardware=ok_json(f'/api/media/{item_id}/playback?transcode=true','POST',{},token)
                    assert request(hardware['stream_url'])[0]==200
                    ok_json(f"/api/playback/{hardware['session_id']}/progress",'PUT',{'position_seconds':0,'duration_seconds':8,'paused':True,'ended':True},token)
                    settings['hardware_acceleration']='none'
                    ok_json('/api/admin/playback/settings','PUT',settings,token)
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
            from accounts_smoke import check_accounts
            check_accounts(request,ok_json,sql,token,config['admin_username'],item_id)
            # A second account cannot mutate another account's playback or reach admin APIs.
            other_id=sql("INSERT INTO users(username,password_hash) VALUES ('other','unused') RETURNING id")
            import hashlib,base64
            other_token=uuid.uuid4().hex
            other_hash=base64.urlsafe_b64encode(hashlib.sha256(other_token.encode()).digest()).decode().rstrip("=")
            sql(f"INSERT INTO sessions(user_id,token_hash,expires_at) VALUES ('{other_id}','{other_hash}',now()+interval '1 hour')")
            assert request(f"/api/playback/{sid}/progress","PUT",progress,other_token)[0] == 404
            assert request(f"/api/playback/{sid}","DELETE",token=other_token)[0]==404
            unloaded=ok_json(f"/api/media/{item_id}/playback","POST",{},token)
            ok_json(f"/api/playback/{unloaded['session_id']}","DELETE",token=token)
            assert request(unloaded['stream_url'])[0]==401
            assert request("/api/admin/stats/users",token=other_token)[0] == 403
            assert request("/api/admin/stats/playback",token=other_token)[0] == 403
            assert request("/api/telemetry/playback-progress","POST",telemetry,other_token)[0] == 404
            assert request('/api/admin/playback/settings',token=other_token)[0]==403
            assert request('/api/admin/playback/settings','PUT',settings,other_token)[0]==403
            assert ok_json("/api/me/list",token=other_token) == []
            # Faza 6: collections and per-account restrictions.
            from collections_smoke import check_collections_and_restrictions
            admin_id=sql(f"SELECT id FROM users WHERE username='{config['admin_username']}'")
            check_collections_and_restrictions(request,ok_json,sql,token,admin_id,item_id,library_id,other_token,other_id)
            # Faza 7: Fix Match validation and original-file downloads.
            from downloads_smoke import check_fix_match_and_downloads
            check_fix_match_and_downloads(request,ok_json,token,item_id,other_token,other_id)
            # Faza 8: cast and crew.
            from people_smoke import check_people
            check_people(request,ok_json,sql,token,item_id,other_token,other_id)
            if args.ffmpeg and args.ffprobe:
                # After the checks that count items and webhook deliveries.
                check_hls_engine(args,root,request,ok_json,sql,token,base)
            ok_json("/api/auth/logout","POST",{},token)
            if args.node:
                event=notification_queue.get(timeout=15)
                while event['type'] in ('scan_progress','library_changed'):
                    event=notification_queue.get(timeout=15)
                assert event['type']=='closed',event
            assert request(resumed["stream_url"])[0] == 401
            assert request("/api/auth/me",token=token)[0] == 401
            # Failed logins are limited per address; once blocked, even the
            # right password gets 429 with Retry-After (no hashing done).
            for _ in range(15):
                status=request('/api/auth/login','POST',{'username':'nobody','password':'wrong-password'})[0]
                if status==429: break
                assert status==401,status
            status,body,headers=request('/api/auth/login','POST',{'username':config['admin_username'],'password':config['admin_password']})
            assert status==429 and int(headers['Retry-After'])>0,(status,body)
            server.terminate()
            server.wait(timeout=10)
            log.close()
            server = None
            print("PASS: migrations, concurrent setup, auth, sealed provider keys, closed CORS, login rate limit, queued scan + progress + stored probe, ranges, resume, ownership, stats, games, ticket revocation, webhook retry, playback settings, remote limits, optional FFmpeg/HLS/NVENC, remux/subtitles/seek/burn-in, browser notifications, collections, parental controls, fix match, downloads, cast and crew")
    finally:
        if server:
            server.terminate()
            server.wait(timeout=10)
        sql("DROP DATABASE " + database + " WITH (FORCE)",args.database_url)

if __name__ == "__main__":
    main()

