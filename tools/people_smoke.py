"""Cast and crew assertions (Faza 8) for the disposable-database
integration suite. Credits are inserted directly (no provider is
contacted); details are marked fresh so the person page does not ask TMDB."""


def check_people(request, ok_json, sql, admin_token, item_id, other_token, other_id):
    person = sql(
        "INSERT INTO people (tmdb_id, name, biography, birth_date, birth_place, imdb_id, details_updated_at) "
        "VALUES (3, 'Harrison Ford', 'An American actor.', '1942-07-13', 'Chicago', 'nm0000148', now()) RETURNING id")
    director = sql("INSERT INTO people (tmdb_id, name, details_updated_at) VALUES (1, 'George Lucas', now()) RETURNING id")
    sql("INSERT INTO media_people (media_item_id, person_id, role, character_name, sort_order) VALUES "
        f"('{item_id}', '{person}', 'Actor', 'Han Solo', 0), ('{item_id}', '{director}', 'Director', NULL, 0)")
    try:
        credits = ok_json(f'/api/media/{item_id}/people', token=other_token)['items']
        assert [(c['name'], c['role'], c['character']) for c in credits] == [
            ('George Lucas', 'Director', None), ('Harrison Ford', 'Actor', 'Han Solo')], credits
        assert all(c['image_url'] is None for c in credits), 'no photo on disk'
        assert request(f'/api/media/{item_id}/people')[0] == 401

        page = ok_json(f'/api/people/{person}', token=other_token)
        assert page['name'] == 'Harrison Ford' and page['birth_date'] == '1942-07-13', page
        assert page['age'] >= 84 and page['death_date'] is None, page
        assert page['imdb_url'] == 'https://www.imdb.com/name/nm0000148/', page
        assert page['tmdb_url'] == 'https://www.themoviedb.org/person/3', page
        assert [(f['item']['id'], f['roles'], f['character']) for f in page['filmography']] == [
            (item_id, ['Actor'], 'Han Solo')], page['filmography']
        assert request(f'/api/people/{person}/image?token={other_token}')[0] == 404, 'no photo yet'
        assert request(f'/api/people/{person}?token={other_token}')[0] == 401, 'JSON needs the header'
        assert request('/api/people/00000000-0000-0000-0000-000000000000', token=other_token)[0] == 404

        # Parental controls: nothing visible, nobody to see.
        ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {'libraries': []}, admin_token)
        assert request(f'/api/people/{person}', token=other_token)[0] == 404
        assert request(f'/api/people/{person}/image?token={other_token}')[0] == 404
        assert request(f'/api/media/{item_id}/people', token=other_token)[0] == 404
        assert ok_json(f'/api/people/{person}', token=admin_token)['name'] == 'Harrison Ford'
    finally:
        ok_json(f'/api/admin/users/{other_id}/restrictions', 'PUT', {}, admin_token)
        sql(f"DELETE FROM people WHERE id IN ('{person}', '{director}')")
