"""Account lifecycle assertions for the disposable-database integration suite."""
import concurrent.futures
import json
import uuid


def check_accounts(request, ok_json, sql, admin_token, admin_name, item_id):
    def login(name, password):
        return ok_json('/api/auth/login', 'POST', {'username': name, 'password': password})

    accounts = ok_json('/api/admin/users', token=admin_token)
    administrator = next(u for u in accounts if u['username'] == admin_name)
    edit_admin = dict(username=admin_name, is_admin=False, disabled=False, ui_language='en')
    assert request(f"/api/admin/users/{administrator['id']}", 'PUT', edit_admin, admin_token)[0] == 409
    edit_admin.update(is_admin=True, disabled=True)
    assert request(f"/api/admin/users/{administrator['id']}", 'PUT', edit_admin, admin_token)[0] == 409
    assert request('/api/admin/users', 'POST', {'username': 'guest'})[0] == 401
    status, body, headers = request('/api/admin/users', 'POST', {'username': 'guest', 'ui_language': 'pl'}, admin_token)
    assert status == 200 and headers['Cache-Control'] == 'no-store'
    created = json.loads(body)
    uid, temporary = created['user']['id'], created['temporary_password']
    assert len(temporary) == 24 and created['user']['must_change_password']
    assert sql(f"SELECT password_hash FROM users WHERE id='{uid}'") != temporary
    assert request('/api/admin/users', 'POST', {'username': 'guest'}, admin_token)[0] == 409
    assert request('/api/admin/users', 'POST', {'username': 'short', 'password': '123'}, admin_token)[0] == 400
    listed = ok_json('/api/admin/users', token=admin_token)
    assert all('password_hash' not in u and 'temporary_password' not in u for u in listed)
    first = login('guest', temporary)
    restricted = first['token']
    assert first['user']['must_change_password']
    assert ok_json('/api/auth/me', token=restricted)['must_change_password']
    assert request('/api/libraries', token=restricted)[0] == 403
    assert request('/api/admin/users', token=restricted)[0] == 403
    assert request(f'/api/media/{item_id}/playback', 'POST', {}, restricted)[0] == 403
    assert request('/api/auth/password', 'POST', {'new_password': temporary}, restricted)[0] == 400
    assert request('/api/auth/password', 'POST', {'new_password': 'short'}, restricted)[0] == 400
    personal = 'Personal-' + uuid.uuid4().hex
    changed = ok_json('/api/auth/password', 'POST', {'new_password': personal}, restricted)
    active = changed['token']
    assert not changed['user']['must_change_password'] and active != restricted
    assert request('/api/auth/me', token=restricted)[0] == 401
    assert request('/api/auth/login', 'POST', {'username': 'guest', 'password': temporary})[0] == 401
    assert request('/api/libraries', token=active)[0] == 200
    assert request('/api/admin/users', 'POST', {'username': 'forbidden'}, active)[0] == 403
    assert request('/api/admin/users', token=active)[0] == 403
    assert request('/api/auth/password', 'POST', {'new_password': 'Another-valid-password'}, active)[0] == 400
    stream = ok_json(f'/api/media/{item_id}/playback', 'POST', {}, active)

    second_temp = ok_json(f'/api/admin/users/{uid}/password', 'POST', {}, admin_token)['temporary_password']
    assert second_temp != temporary
    assert request('/api/auth/me', token=active)[0] == 401
    assert request(stream['stream_url'])[0] == 401
    assert request('/api/auth/login', 'POST', {'username': 'guest', 'password': personal})[0] == 401
    limited_a = login('guest', second_temp)['token']
    limited_b = login('guest', second_temp)['token']
    # Two attempts to consume the same temporary credential cannot both succeed.
    passwords = ['Own-' + uuid.uuid4().hex, 'Own-' + uuid.uuid4().hex]
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
        futures = [workers.submit(request, '/api/auth/password', 'POST', {'new_password': p}, t)
                   for p, t in zip(passwords, [limited_a, limited_b])]
        results = [f.result() for f in futures]
    assert sum(r[0] == 200 for r in results) == 1
    assert all(r[0] in (200, 401, 409) for r in results)
    winner = next(i for i, r in enumerate(results) if r[0] == 200)
    personal = passwords[winner]
    active = json.loads(results[winner][1])['token']
    profile = dict(username='guest-renamed', is_admin=True, disabled=False, ui_language='pl')
    ok_json(f'/api/admin/users/{uid}', 'PUT', profile, admin_token)
    assert request('/api/auth/me', token=active)[0] == 401
    active = login('guest-renamed', personal)['token']
    assert request('/api/admin/users', token=active)[0] == 200
    profile.update(is_admin=False, disabled=True)
    ok_json(f'/api/admin/users/{uid}', 'PUT', profile, admin_token)
    assert request('/api/auth/me', token=active)[0] == 401
    assert request('/api/auth/login', 'POST', {'username': 'guest-renamed', 'password': personal})[0] == 401
    profile['disabled'] = False
    ok_json(f'/api/admin/users/{uid}', 'PUT', profile, admin_token)
    active = login('guest-renamed', personal)['token']
    assert request('/api/admin/users', token=active)[0] == 403
    replacement = 'Final-' + uuid.uuid4().hex
    result = ok_json('/api/auth/password', 'POST', {'current_password': personal, 'new_password': replacement}, active)
    assert request('/api/auth/me', token=active)[0] == 401
    assert request('/api/libraries', token=result['token'])[0] == 200
    print('PASS: admin-only accounts, generated credentials, enforced password change, reset/revocation, concurrent password changes, roles, disable/enable, last admin')
