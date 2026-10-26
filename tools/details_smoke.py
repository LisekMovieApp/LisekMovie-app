"""Detail page extras and chapters (Faza 9) for the disposable-database
integration suite. Provider data and chapters are written directly; with
FFmpeg a chapter thumbnail is cut from the scanned file."""


def check_details(request, ok_json, sql, admin_token, item_id, other_token, other_id, has_ffmpeg):
    sql("UPDATE media_items SET metadata = jsonb_set(metadata, '{provider_data}', "
        "coalesce(metadata->'provider_data', '{}'::jsonb) || '{\"tagline\": \"Jak szybko lubisz?\", "
        "\"original_title\": \"2 Fast 2 Furious\", \"rating\": 6.5, \"runtime_minutes\": 108, "
        "\"genres\": [\"Akcja\"], \"tags\": [\"car race\"], \"studios\": [\"Universal Pictures\"], "
        "\"imdb_id\": \"tt0322259\", \"tmdb_id\": 584}'::jsonb) "
        f"WHERE id = '{item_id}'")
    sql(f"INSERT INTO media_chapters (media_item_id, chapter_index, start_seconds, end_seconds, title) VALUES "
        f"('{item_id}', 0, 0, 4, NULL), ('{item_id}', 1, 4, 8, 'Pościg')")
    try:
        details = ok_json(f'/api/media/{item_id}/details', token=other_token)
        assert details['tagline'] == 'Jak szybko lubisz?', details
        assert details['original_title'] == '2 Fast 2 Furious'
        assert details['rating'] == 6.5 and details['genres'] == ['Akcja'] and details['tags'] == ['car race']
        assert [l['name'] for l in details['external_links']] == ['IMDb', 'TMDB'], details['external_links']
        assert details['chapters_generated'] is False
        assert [(c['index'], c['title'], c['start_seconds']) for c in details['chapters']] == [
            (0, None, 0.0), (1, 'Pościg', 4.0)], details['chapters']
        image = details['chapters'][1]['image_url']
        assert image == f'/api/media/{item_id}/chapters/1/image?v=4000', image
        if has_ffmpeg:
            # The scanned file was probed: its streams are offered for playback.
            assert details['video']['label'].endswith('H264'), details['video']
            assert details['runtime_minutes'] is not None
            status, body, headers = request(f'{image}&token={other_token}')
            assert status == 200 and body[:2] == b'\xff\xd8', (status, body[:20])
            assert 'immutable' in headers['Cache-Control']
            # Second time from the cache.
            assert request(f'{image}&token={other_token}')[1] == body
        assert request(f'/api/media/{item_id}/chapters/9/image?token={other_token}')[0] == 404
        assert request(f'/api/media/{item_id}/details?token={other_token}')[0] == 401, 'JSON needs the header'
        # Parental controls: nothing visible, no details or scenes.
        ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {'libraries': []}, admin_token)
        assert request(f'/api/media/{item_id}/details', token=other_token)[0] == 404
        assert request(f'{image}&token={other_token}')[0] == 404
    finally:
        ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {}, admin_token)
        sql(f"DELETE FROM media_chapters WHERE media_item_id = '{item_id}'")
