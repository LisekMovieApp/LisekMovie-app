// Client-side calls for Faza 5 completion's three media-item admin
// surfaces (`/api/admin/libraries/{id}/items`, `/api/admin/media/*`,
// `/api/admin/libraries/{id}/provider-order/*` on the backend): the
// library items browse view, the advanced metadata editor, the fallback
// warning view, and per-library provider-order drag & drop. Same split as
// `libraries.ts`/`providers.ts`: pure request-shape logic here, unit-tested
// without rendering Svelte.

import { buildAuthHeader } from '$lib/auth';
import { AdminApiError, assertOk } from './api';

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
	/** Faza 6: the administrator's own age rating (parental controls);
	 * `null` = the provider's rating applies. */
	age_rating?: number | null;
	provider_age_rating?: number | null;
	certification?: string | null;
	/** Faza 20: the series and episode side (null for a film). */
	series_id?: string | null;
	series_title?: string | null;
	episode_count?: number | null;
	episode_title?: string | null;
	episode_overview?: string | null;
	episode_air_date?: string | null;
	/** Cache-buster for artwork previews. */
	version?: number;
}

/** What a save edits: a film itself, a whole series, or one episode. */
export type EditScope = 'item' | 'series' | 'episode';
export type ArtworkKind = 'poster' | 'backdrop' | 'logo' | 'thumb';

/** The metadata editor's save payload — see
 * `admin::UpdateMediaItemRequest` on the backend. `locked_fields` here is
 * the *unchecked* boxes in the editor's "Pola aktywne" section (see that
 * section's own doc comment in `MediaItemEditorModal.svelte`). */
export interface UpdateMediaItemPayload {
	scope?: EditScope;
	title?: string;
	original_title?: string | null;
	year?: number | null;
	overview?: string | null;
	metadata_locked?: boolean;
	forced_provider?: string | null;
	locked_fields: string[];
	/** Faza 6: `null` removes the administrator's own age rating. */
	age_rating?: number | null;
	episode_title?: string | null;
	episode_overview?: string | null;
	episode_air_date?: string | null;
}

export interface UpdateMediaItemResult {
	ok: boolean;
	updated: number;
	/** Fields the server locked because this save changed them. */
	auto_locked: string[];
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
): Promise<UpdateMediaItemResult> {
	const response = await fetch(`/api/admin/media/${encodeURIComponent(itemId)}`, {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify(payload)
	});
	await assertOk(response);
	return (await response.json()) as UpdateMediaItemResult;
}

/** Replaces one picture with an uploaded file or the picture at `source`
 * (an http(s) link). Returns the image URL with a cache-buster. */
export async function setArtwork(
	token: string,
	itemId: string,
	kind: ArtworkKind,
	source: Blob | string
): Promise<string> {
	const body = typeof source === 'string' ? JSON.stringify({ url: source }) : source;
	const contentType =
		typeof source === 'string' ? 'application/json' : source.type || 'application/octet-stream';
	const response = await fetch(
		`/api/admin/media/${encodeURIComponent(itemId)}/artwork/${encodeURIComponent(kind)}`,
		{ method: 'PUT', headers: { ...buildAuthHeader(token), 'Content-Type': contentType }, body }
	);
	await assertOk(response);
	return ((await response.json()) as { url: string }).url;
}

/** Translation key for a failed editor request. */
export function editorErrorKey(error: unknown, fallback = 'admin.editor_save_failed'): string {
	const code = error instanceof AdminApiError ? error.code : null;
	const known = new Set([
		'series_title_taken',
		'title_empty',
		'invalid_year',
		'invalid_air_date',
		'not_an_episode',
		'artwork_invalid_image',
		'artwork_invalid_url',
		'artwork_too_large',
		'artwork_download_failed',
		'artwork_empty'
	]);
	return code && known.has(code) ? `admin.editor_errors.${code}` : fallback;
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
