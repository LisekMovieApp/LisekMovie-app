// Client-side calls for Faza 5 completion's three media-item admin
// surfaces (`/api/admin/libraries/{id}/items`, `/api/admin/media/*`,
// `/api/admin/libraries/{id}/provider-order/*` on the backend): the
// library items browse view, the advanced metadata editor, the fallback
// warning view, and per-library provider-order drag & drop. Same split as
// `libraries.ts`/`providers.ts`: pure request-shape logic here, unit-tested
// without rendering Svelte.

import { buildAuthHeader } from '$lib/auth';
import { assertOk } from './api';

/** One row in a library's items browse list — see
 * `lisek_db::MediaItemSummary`'s own doc comment for what each field means
 * server-side. */
export interface MediaItemSummary {
	id: string;
	item_type: string;
	title: string;
	year: number | null;
	season: number | null;
	episode: number | null;
	metadata_locked: boolean;
	metadata_fallback: boolean;
}

/** One item's full editable shape — what the metadata editor loads when it
 * opens and what it locally mutates before saving. Mirrors
 * `lisek_db::MediaItemDetail` field-for-field. */
export interface MediaItemDetail {
	id: string;
	library_id: string;
	item_type: string;
	title: string;
	original_title: string | null;
	year: number | null;
	overview: string | null;
	season: number | null;
	episode: number | null;
	metadata_locked: boolean;
	forced_provider: string | null;
	locked_fields: string[];
}

/** The metadata editor's save payload — see
 * `admin::UpdateMediaItemRequest` on the backend. `locked_fields` here is
 * the *unchecked* boxes in the editor's "Pola aktywne" section (see that
 * section's own doc comment in `MediaItemEditorModal.svelte`). */
export interface UpdateMediaItemPayload {
	title: string;
	original_title: string | null;
	year: number | null;
	overview: string | null;
	metadata_locked: boolean;
	forced_provider: string | null;
	locked_fields: string[];
}

/** One item currently flagged `metadata_fallback` — the fallback warning
 * view's row shape. Mirrors `lisek_db::FallbackItem`. */
export interface FallbackItem {
	id: string;
	library_id: string;
	title: string;
	file_path: string;
}

export interface MigrateFallbackResult {
	poster_migrated: boolean;
	backdrop_migrated: boolean;
}

export async function fetchLibraryItems(
	token: string,
	libraryId: string
): Promise<MediaItemSummary[]> {
	const response = await fetch(`/api/admin/libraries/${encodeURIComponent(libraryId)}/items`, {
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
	return (await response.json()) as MediaItemSummary[];
}

export async function fetchMediaItem(token: string, itemId: string): Promise<MediaItemDetail> {
	const response = await fetch(`/api/admin/media/${encodeURIComponent(itemId)}`, {
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
	return (await response.json()) as MediaItemDetail;
}

export async function updateMediaItem(
	token: string,
	itemId: string,
	payload: UpdateMediaItemPayload
): Promise<void> {
	const response = await fetch(`/api/admin/media/${encodeURIComponent(itemId)}`, {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify(payload)
	});
	await assertOk(response);
}

export async function fetchFallbackItems(token: string): Promise<FallbackItem[]> {
	const response = await fetch('/api/admin/media/fallback', { headers: buildAuthHeader(token) });
	await assertOk(response);
	return (await response.json()) as FallbackItem[];
}

export async function migrateFallbackItem(
	token: string,
	itemId: string
): Promise<MigrateFallbackResult> {
	const response = await fetch(
		`/api/admin/media/${encodeURIComponent(itemId)}/migrate-to-media-folder`,
		{ method: 'POST', headers: buildAuthHeader(token) }
	);
	await assertOk(response);
	return (await response.json()) as MigrateFallbackResult;
}

export async function fetchProviderOrder(
	token: string,
	libraryId: string,
	itemType: string
): Promise<string[]> {
	const response = await fetch(
		`/api/admin/libraries/${encodeURIComponent(libraryId)}/provider-order/${encodeURIComponent(itemType)}`,
		{ headers: buildAuthHeader(token) }
	);
	await assertOk(response);
	const body = (await response.json()) as { provider_order: string[] };
	return body.provider_order;
}

export async function setProviderOrder(
	token: string,
	libraryId: string,
	itemType: string,
	providerOrder: string[]
): Promise<void> {
	const response = await fetch(
		`/api/admin/libraries/${encodeURIComponent(libraryId)}/provider-order/${encodeURIComponent(itemType)}`,
		{
			method: 'PUT',
			headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
			body: JSON.stringify({ provider_order: providerOrder })
		}
	);
	await assertOk(response);
}
