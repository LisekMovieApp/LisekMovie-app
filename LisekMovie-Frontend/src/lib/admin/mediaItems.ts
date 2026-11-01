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
	/** Faza 21: the film's / series' score (0-10) and the directors typed
	 * in the editor (empty: the provider's credits). */
	rating?: number | null;
	directors?: string[];
	/** Faza 21: the episode's own score and directors. */
	episode_rating?: number | null;
	episode_directors?: string[];
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
	/** Faza 21: `null` clears the score, a missing key keeps it. */
	rating?: number | null;
	directors?: string[];
	episode_rating?: number | null;
	episode_directors?: string[];
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

/** Faza 21: one season as the season editor loads it — see
 * `lisek_db::editor::SeasonDetail`. */
export interface SeasonDetail {
	series_id: string;
	series_title: string;
	season: number;
	name: string | null;
	overview: string | null;
	air_year: number | null;
	/** `name`, `overview`, `air_year`, `poster`. */
	locked_fields: string[];
	episode_count: number;
	/** Cache-buster for the poster preview. */
	version: number;
}

export interface UpdateSeasonPayload {
	name: string | null;
	overview: string | null;
	air_year: number | null;
	locked_fields: string[];
}

/** `itemId` is any episode of the series. */
export async function fetchSeason(token: string, itemId: string, season: number): Promise<SeasonDetail> {
	const response = await fetch(
		`/api/admin/media/${encodeURIComponent(itemId)}/seasons/${season}`,
		{ headers: buildAuthHeader(token) }
	);
	await assertOk(response);
	return (await response.json()) as SeasonDetail;
}

export async function updateSeason(
	token: string,
	itemId: string,
	season: number,
	payload: UpdateSeasonPayload
): Promise<UpdateMediaItemResult> {
	const response = await fetch(
		`/api/admin/media/${encodeURIComponent(itemId)}/seasons/${season}`,
		{
			method: 'PUT',
			headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
			body: JSON.stringify(payload)
		}
	);
	await assertOk(response);
	return (await response.json()) as UpdateMediaItemResult;
}

/** Replaces a season's poster; returns its URL with a cache-buster. */
export async function setSeasonPoster(
	token: string,
	itemId: string,
	season: number,
	source: Blob | string
): Promise<string> {
	const body = typeof source === 'string' ? JSON.stringify({ url: source }) : source;
	const contentType =
		typeof source === 'string' ? 'application/json' : source.type || 'application/octet-stream';
	const response = await fetch(
		`/api/admin/media/${encodeURIComponent(itemId)}/seasons/${season}/poster`,
		{ method: 'PUT', headers: { ...buildAuthHeader(token), 'Content-Type': contentType }, body }
	);
	await assertOk(response);
	return ((await response.json()) as { url: string }).url;
}

/** Faza 21: a score typed as "7,5" or "7.5"; `null` for empty, `undefined`
 * for anything that is not a number from 0 to 10. */
export function parseRating(text: string): number | null | undefined {
	const value = text.trim().replace(',', '.');
	if (value === '') return null;
	if (!/^\d{1,2}(\.\d+)?$/.test(value)) return undefined;
	const number = Number(value);
	return number >= 0 && number <= 10 ? Math.round(number * 10) / 10 : undefined;
}

/** Faza 21: directors typed one per line or separated by commas. */
export function parseNames(text: string): string[] {
	const seen = new Set<string>();
	return text
		.split(/[\n,]/)
		.map((name) => name.trim())
		.filter((name) => name !== '' && !seen.has(name.toLowerCase()) && seen.add(name.toLowerCase()))
		.slice(0, 20);
}

/** Translation key for a failed editor request. */
export function editorErrorKey(error: unknown, fallback = 'admin.editor_save_failed'): string {
	const code = error instanceof AdminApiError ? error.code : null;
	const known = new Set([
		'series_title_taken',
		'title_empty',
		'invalid_year',
		'invalid_rating',
		'invalid_input',
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
