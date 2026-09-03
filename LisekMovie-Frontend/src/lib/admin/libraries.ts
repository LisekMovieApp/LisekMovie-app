// Client-side calls for the Admin Panel's libraries tab
// (`/api/admin/libraries/*` on the backend). Same split as `providers.ts`:
// pure request-shape logic here, unit-tested without rendering Svelte.

import { buildAuthHeader } from '$lib/auth';
import { assertOk } from './api';

export type LibraryType = 'movies' | 'tv_shows';

export interface Library {
	id: string;
	name: string;
	library_type: LibraryType;
	sort_order: number;
	root_paths: string[];
	/** Whether a metadata match's poster/backdrop are written directly next
	 * to the video file rather than only into the
	 * server's own local cache. Defaults to `true` server-side; see
	 * `AddLibraryModal`'s checkbox for where an admin can turn it off. */
	save_metadata_in_media_folder: boolean;
}

export interface ScanSummary {
	movies_found: number;
	episodes_found: number;
	unrecognized: number;
	/** How many scanned items matched an enabled, configured metadata
	 * provider this scan. `0` whenever no provider has both an API key set
	 * and its "Enabled" toggle on — see `images_saved`'s own comment. */
	metadata_matched: number;
	/** How many items actually got a poster and/or backdrop saved to disk
	 * this scan (media folder or local cache) — can be lower than
	 * `metadata_matched` when a match had no usable image. A library page
	 * showing `0` here alongside a nonzero `movies_found`/`episodes_found`
	 * is the signal to check the provider's API key, not a scanner bug. */
	images_saved: number;
}

export async function fetchLibraries(token: string): Promise<Library[]> {
	const response = await fetch('/api/admin/libraries', { headers: buildAuthHeader(token) });
	await assertOk(response);
	return (await response.json()) as Library[];
}

export async function createLibrary(
	token: string,
	name: string,
	libraryType: LibraryType,
	rootPaths: string[],
	saveMetadataInMediaFolder: boolean = true
): Promise<Library> {
	const response = await fetch('/api/admin/libraries', {
		method: 'POST',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({
			name,
			library_type: libraryType,
			root_paths: rootPaths,
			save_metadata_in_media_folder: saveMetadataInMediaFolder
		})
	});
	await assertOk(response);
	return (await response.json()) as Library;
}

export async function deleteLibrary(token: string, libraryId: string): Promise<void> {
	const response = await fetch(`/api/admin/libraries/${encodeURIComponent(libraryId)}`, {
		method: 'DELETE',
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
}

export async function scanLibrary(token: string, libraryId: string): Promise<ScanSummary> {
	const response = await fetch(`/api/admin/libraries/${encodeURIComponent(libraryId)}/scan`, {
		method: 'POST',
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
	return (await response.json()) as ScanSummary;
}

/** Nudges one library up or down in the shared `sort_order` — the "W
 * górę"/"W dół" buttons on the libraries tab (Runda 9). Swaps with the
 * immediate neighbor server-side (see `lisek_db::move_library`'s doc
 * comment); the caller re-fetches the list afterwards rather than trying to
 * reorder its local copy, so it can never drift from what the backend
 * actually did. */
export async function moveLibrary(
	token: string,
	libraryId: string,
	direction: 'up' | 'down'
): Promise<void> {
	const response = await fetch(`/api/admin/libraries/${encodeURIComponent(libraryId)}/reorder`, {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ direction })
	});
	await assertOk(response);
}
