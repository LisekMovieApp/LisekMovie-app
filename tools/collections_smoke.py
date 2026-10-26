"""Collections and parental-control assertions (Faza 6) for the
disposable-database integration suite."""


def check_collections_and_restrictions(request, ok_json, sql, admin_token, admin_id, item_id, library_id, other_token, other_id):
    # A second film next to the scanned one; both join one collection the
    # way metadata enrichment would link them.
    sequel = sql(
        "INSERT INTO media_items (library_id, item_type, title, file_path, metadata) "
        f"SELECT library_id, item_type, title || ' II', file_path || '.sequel', "
        f"'{{\"year\": 2031}}'::jsonb FROM media_items WHERE id = '{item_id}' RETURNING id")
    collection = sql("INSERT INTO collections (provider, provider_id, name) VALUES ('tmdb', '42', 'Smoke Saga') RETURNING id")
    sql(f"INSERT INTO collection_items (collection_id, media_item_id) VALUES ('{collection}', '{item_id}'), ('{collection}', '{sequel}')")
    try:
        row = ok_json(f'/api/libraries/{library_id}/collections', token=admin_token)
        assert [(c['id'], c['item_type'], c['item_count']) for c in row['items']] == [(collection, 'collection', 2)], row
        assert row['items'][0]['poster_url'] == f'/api/collections/{collection}/poster'
        grouped = ok_json(f'/api/libraries/{library_id}/items?group_collections=true', token=admin_token)
        assert grouped['total'] == 1 and grouped['items'][0]['item_type'] == 'collection', grouped
        flat = ok_json(f'/api/libraries/{library_id}/items', token=admin_token)
        assert flat['total'] == 2, flat
        detail = ok_json(f'/api/collections/{collection}', token=admin_token)
        assert detail['name'] == 'Smoke Saga' and detail['item_count'] == 2
        years = [i['year'] for i in detail['items']]
        assert years == sorted(years, key=lambda y: (y is None, y or 0)), 'release order, undated last'
        assert ok_json(f'/api/media/{item_id}/collection', token=admin_token)['id'] == collection
        assert request(f'/api/collections/{collection}/poster')[0] == 401
        assert request(f'/api/collections/{collection}/poster?token={admin_token}')[0] in (200, 404)
        assert request(f'/api/collections/{collection}?token={admin_token}')[0] == 401, 'JSON needs the header'

        # Parental controls: the other account up to 12, unrated hidden.
        assert request(f'/api/admin/users/{other_id}/restrictions', token=other_token)[0] == 403
        assert request(f'/api/admin/users/{other_id}/restrictions', 'PUT', {'max_age_rating': 30}, admin_token)[0] == 400
        saved = ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT',
                        {'libraries': None, 'max_age_rating': 12, 'block_unrated': True}, admin_token)
        assert saved == {'libraries': None, 'max_age_rating': 12, 'block_unrated': True, 'can_download': True, 'enforced': True}, saved
        assert [r['user_id'] for r in ok_json('/api/admin/user-restrictions', token=admin_token)] == [other_id]
        assert request(f'/api/media/{item_id}', token=other_token)[0] == 404
        assert request(f'/api/media/{item_id}/playback', 'POST', {}, other_token)[0] == 404
        assert request(f'/api/media/{item_id}/poster?token={other_token}')[0] == 404
        assert request(f'/api/collections/{collection}', token=other_token)[0] == 404
        assert ok_json(f'/api/libraries/{library_id}/items', token=other_token)['total'] == 0
        assert ok_json('/api/home', token=other_token)['recently_added'] == []
        # Rated 7 (an administrator's own value): visible again, with its badge.
        sql(f"UPDATE media_items SET metadata = metadata || '{{\"age_rating\": 7}}' WHERE id = '{item_id}'")
        shown = ok_json(f'/api/media/{item_id}', token=other_token)
        assert shown['age_rating'] == 7 and shown.get('certification') is None, shown
        assert ok_json(f'/api/collections/{collection}', token=other_token)['item_count'] == 1
        assert ok_json(f'/api/libraries/{library_id}/collections', token=other_token)['items'] == [], 'one film is not a group'
        # Only another library: this one disappears entirely.
        ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {'libraries': []}, admin_token)
        assert ok_json('/api/libraries', token=other_token) == []
        assert request(f'/api/libraries/{library_id}/items', token=other_token)[0] == 404
        assert request(f'/api/media/{item_id}', token=other_token)[0] == 404
        # Administrators are never restricted.
        assert ok_json(f'/api/admin/users/{admin_id}/restrictions', 'PUT', {'libraries': []}, admin_token)['enforced'] is False
        assert request(f'/api/media/{item_id}', token=admin_token)[0] == 200
        # Cleared: everything is back.
        cleared = ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {}, admin_token)
        assert cleared['libraries'] is None and cleared['max_age_rating'] is None
        assert sql(f"SELECT count(*) FROM user_restrictions WHERE user_id = '{other_id}'") == '0'
        assert request(f'/api/media/{item_id}', token=other_token)[0] == 200
    finally:
        ok_json(f'/api/admin/users/{admin_id}/restrictions', 'PUT', {}, admin_token)
        sql(f"UPDATE media_items SET metadata = metadata - 'age_rating' WHERE id = '{item_id}'")
        sql(f"DELETE FROM media_items WHERE id = '{sequel}'")
        sql(f"DELETE FROM collections WHERE id = '{collection}'")
