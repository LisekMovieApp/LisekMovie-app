"""Faza 25 for the disposable-database integration suite: a library edited
in place — renamed, a second folder (a new disk) added and scanned, a
folder removed (its items leave the library, the files stay)."""

import shutil

from phase20_smoke import scan


def check_library_edit(request, ok_json, sql, token, other_token, root, movie_path):
    first = root / "dysk1" / "Filmy"
    second = root / "dysk2" / "Filmy"
    first.mkdir(parents=True)
    second.mkdir(parents=True)
    shutil.copy(movie_path, first / "Pierwszy (2001).mp4")
    shutil.copy(movie_path, second / "Drugi (2002).mp4")
    library = ok_json("/api/admin/libraries", "POST", {
        "name": "Filmy", "library_type": "movies", "root_paths": [f"{first}/"],
        "save_metadata_in_media_folder": False}, token)
    assert library["root_paths"] == [str(first)], library
    try:
        scan(request, ok_json, token, library["id"])
        titles = lambda: sql(f"SELECT string_agg(title, ',' ORDER BY title) FROM media_items WHERE library_id = '{library['id']}'")
        assert titles() == "Pierwszy"

        # A new disk: one more folder, a new name; missing folders are reported.
        answer = ok_json(f"/api/admin/libraries/{library['id']}", "PUT", {
            "name": " Wszystkie filmy ", "root_paths": [str(first), f" {second}// ", str(first), "/nie/ma/takiego"]}, token)
        assert answer["library"]["name"] == "Wszystkie filmy"
        assert answer["library"]["root_paths"] == [str(first), str(second), "/nie/ma/takiego"], answer
        assert answer["missing_paths"] == ["/nie/ma/takiego"] and answer["removed_items"] == 0, answer
        scan(request, ok_json, token, library["id"])
        assert titles() == "Drugi,Pierwszy"

        # Bad edits.
        path = f"/api/admin/libraries/{library['id']}"
        assert request(path, "PUT", {"root_paths": ["Filmy"]}, token)[0] == 400
        assert request(path, "PUT", {"root_paths": []}, token)[0] == 400
        assert request(path, "PUT", {"name": "  "}, token)[0] == 400
        assert request(path, "PUT", {"name": "X"}, other_token)[0] == 403
        assert request("/api/admin/libraries/00000000-0000-0000-0000-000000000000", "PUT", {"name": "X"}, token)[0] == 404

        # The first disk goes: its film leaves the library, its file stays.
        answer = ok_json(path, "PUT", {"root_paths": [str(second)]}, token)
        assert answer["removed_items"] == 1 and answer["library"]["name"] == "Wszystkie filmy", answer
        assert titles() == "Drugi"
        assert (first / "Pierwszy (2001).mp4").exists()
        listed = next(l for l in ok_json("/api/admin/libraries", token=token) if l["id"] == library["id"])
        assert listed["root_paths"] == [str(second)], listed
    finally:
        request(f"/api/admin/libraries/{library['id']}", "DELETE", None, token)
