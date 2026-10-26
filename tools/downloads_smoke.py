"""Fix Match and download assertions (Faza 7) for the disposable-database
integration suite. No provider is contacted: only the request validation
of the Fix Match endpoints runs here."""

import urllib.parse


def check_fix_match_and_downloads(request, ok_json, admin_token, item_id, other_token, other_id):
    # Fix Match is an administrator tool and validates before it searches.
    assert request('/api/providers/search?query=Alien&kind=movie', token=other_token)[0] == 403
    status, body, _ = request('/api/providers/search?query=%20&kind=movie', token=admin_token)
    assert status == 400 and b'invalid_query' in body, (status, body)
    status, body, _ = request('/api/providers/search?query=Alien&kind=book', token=admin_token)
    assert status == 400 and b'invalid_kind' in body, (status, body)
    status, body, _ = request('/api/providers/search?query=Alien&kind=movie&provider=nope', token=admin_token)
    assert status == 400 and b'provider_not_configured' in body, (status, body)
    assert request(f'/api/media/{item_id}/identify', 'POST', {'provider': 'tmdb', 'provider_id': '1'}, other_token)[0] == 403
    status, body, _ = request(f'/api/media/{item_id}/identify', 'POST', {'provider': 'tmdb', 'provider_id': ' '}, admin_token)
    assert status == 400 and b'invalid_provider_id' in body, (status, body)

    # Downloads: a short-lived ticket in the URL, because a browser download
    # cannot send the Authorization header.
    assert ok_json('/api/me/access', token=other_token) == {'can_download': True, 'restricted': False}
    ticket = ok_json(f'/api/media/{item_id}/download-ticket', 'POST', {}, other_token)
    assert ticket['url'].startswith(f'/api/media/{item_id}/download?ticket='), ticket
    assert ticket['size_bytes'] > 10 and ticket['expires_in'] > 0, ticket
    status, body, headers = request(ticket['url'])
    assert status == 200 and len(body) == ticket['size_bytes'], (status, len(body))
    disposition = headers['Content-Disposition']
    assert disposition.startswith('attachment; filename="') and "filename*=UTF-8''" in disposition, disposition
    assert urllib.parse.quote(ticket['file_name']) in disposition, (ticket, disposition)
    status, body, headers = request(ticket['url'], headers={'Range': 'bytes=0-9'})
    assert status == 206 and len(body) == 10, (status, len(body))
    assert headers['Content-Range'] == f"bytes 0-9/{ticket['size_bytes']}", headers['Content-Range']
    assert request(f'/api/media/{item_id}/download')[0] in (400, 401), 'the ticket is required'
    assert request(f'/api/media/{item_id}/download?ticket=forged')[0] == 401
    other_item = '00000000-0000-0000-0000-000000000000'
    query = ticket['url'].split('?', 1)[1]
    assert request(f'/api/media/{other_item}/download?{query}')[0] == 401, 'bound to one item'

    # Switched off: no new ticket, and the one already issued stops working.
    try:
        saved = ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {'can_download': False}, admin_token)
        assert saved['can_download'] is False and saved['libraries'] is None, saved
        assert ok_json('/api/me/access', token=other_token) == {'can_download': False, 'restricted': False}, 'the catalogue is not filtered'
        status, body, _ = request(f'/api/media/{item_id}/download-ticket', 'POST', {}, other_token)
        assert status == 403 and b'downloads_disabled' in body, (status, body)
        assert request(ticket['url'])[0] == 403
        assert request(f'/api/media/{item_id}', token=other_token)[0] == 200, 'browsing stays open'
        # Administrators can always download.
        assert ok_json(f'/api/media/{item_id}/download-ticket', 'POST', {}, admin_token)['size_bytes'] == ticket['size_bytes']
    finally:
        ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {}, admin_token)
