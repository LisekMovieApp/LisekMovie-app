"""Profiles (Faza 19) for the disposable-database integration suite: avatar
upload / serving / removal, the account's own UI language, the server's
default language in /api/auth/me, the keep-alive and the online status in
the admin user list."""

import base64
import hashlib
import json
import struct
import urllib.error
import urllib.request
import zlib


def png(width, height, rgb):
    """A plain RGB PNG, without any imaging library."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    row = b"\x00" + bytes(rgb) * width
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(row * height)) + chunk(b"IEND", b""))


def upload(base, token, body, content_type="image/png"):
    req = urllib.request.Request(base + "/api/users/me/avatar", method="POST", data=body,
                                 headers={"Authorization": f'LisekMovie Token="{token}"', "Content-Type": content_type})
    try:
        response = urllib.request.urlopen(req, timeout=60)
    except urllib.error.HTTPError as error:
        response = error
    return response.status, response.read()


def check_profiles(request, ok_json, sql, base, token, other_token, root):
    me = ok_json("/api/auth/me", token=token)
    assert me["avatar_url"] is None and me["server_language"] == "en", me
    user_id = me["id"]

    # Upload: any size and shape becomes a 256x256 JPEG under config/avatars.
    status, body = upload(base, token, png(640, 360, (200, 40, 40)))
    assert status == 200, (status, body)
    avatar_url = json.loads(body)["avatar_url"]
    assert avatar_url.startswith(f"/api/users/{user_id}/avatar?v="), avatar_url
    stored = root / "config" / "avatars" / f"{user_id}.jpg"
    assert stored.read_bytes()[:3] == b"\xff\xd8\xff"
    assert ok_json("/api/auth/me", token=token)["avatar_url"] == avatar_url

    # Served as an image to any signed-in account, `?token=` allowed for <img>.
    status, image, headers = request(avatar_url + f"&token={other_token}")
    assert status == 200 and headers["Content-Type"] == "image/jpeg" and image == stored.read_bytes(), status
    assert request(avatar_url)[0] == 401
    assert request(avatar_url, headers={"If-None-Match": headers["ETag"]}, token=token)[0] == 304

    # Rejected uploads leave the avatar alone.
    assert upload(base, token, b"")[0] == 400
    status, body = upload(base, token, b"<svg xmlns='http://www.w3.org/2000/svg'/>", "image/svg+xml")
    assert status == 400 and json.loads(body)["error_code"] == "avatar_invalid_image", body
    assert upload(base, token, b"\x00" * (10 * 1024 * 1024 + 1))[0] == 413
    assert ok_json("/api/auth/me", token=token)["avatar_url"] == avatar_url

    # Admin user list: avatars and presence.
    other_hash = base64.urlsafe_b64encode(hashlib.sha256(other_token.encode()).digest()).decode().rstrip("=")
    assert request("/api/users/me/heartbeat", "POST", {}, other_token)[0] == 204
    listing = {u["username"]: u for u in ok_json("/api/admin/users", token=token)}
    assert listing["test-admin"]["avatar_url"] == avatar_url and listing["test-admin"]["online"], listing["test-admin"]
    assert listing["other"]["online"] and listing["other"]["avatar_url"] is None, listing["other"]
    assert request("/api/admin/users", token=other_token)[0] == 403
    sql(f"UPDATE sessions SET last_seen_at = now() - interval '1 hour' WHERE token_hash = '{other_hash}'")
    listing = {u["username"]: u for u in ok_json("/api/admin/users", token=token)}
    assert not listing["other"]["online"] and listing["other"]["last_seen_at"], listing["other"]

    # Own UI language (the server default stays).
    assert ok_json("/api/users/me/language", "PUT", {"ui_language": "pl"}, other_token)["ui_language"] == "pl"
    other = ok_json("/api/auth/me", token=other_token)
    assert other["ui_language"] == "pl" and other["server_language"] == "en", other
    assert request("/api/users/me/language", "PUT", {"ui_language": "de"}, other_token)[0] == 400
    assert sql("SELECT default_language FROM server_settings") == "en"

    # Removal.
    assert ok_json("/api/users/me/avatar", "DELETE", None, token)["avatar_url"] is None
    assert request(avatar_url, token=token)[0] == 404
    assert not stored.exists()
    assert ok_json("/api/auth/me", token=token)["avatar_url"] is None
