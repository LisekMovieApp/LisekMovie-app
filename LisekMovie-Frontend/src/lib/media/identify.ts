// "Fix match" (Faza 7): search the metadata providers by hand and pin the
// right entry on a film or a whole series. Administrators only.

import type { components } from '$lib/api/schema';
import { api } from '$lib/playback/api';

export type ProviderCandidate = components['schemas']['ProviderCandidateResponse'];
export type ProviderSearch = components['schemas']['ProviderSearchResponse'];
export type IdentifyResult = components['schemas']['IdentifyResponse'];
export type MatchKind = 'movie' | 'series';

export function searchProviders(
	token: string,
	kind: MatchKind,
	query: string,
	year: number | null,
	provider: string | null = null
): Promise<ProviderSearch> {
	const params = new URLSearchParams({ kind, query: query.trim() });
	if (year !== null) params.set('year', String(year));
	if (provider) params.set('provider', provider);
	return api<ProviderSearch>(token, `/api/providers/search?${params}`);
}

export function identifyItem(
	token: string,
	itemId: string,
	provider: string,
	providerId: string
): Promise<IdentifyResult> {
	return api<IdentifyResult>(
		token,
		`/api/media/${encodeURIComponent(itemId)}/identify`,
		'POST',
		{ provider, provider_id: providerId },
		false
	);
}

/** A year typed into the search form: four digits, else none. */
export function parseYear(raw: string): number | null {
	const value = raw.trim();
	return /^\d{4}$/.test(value) ? Number(value) : null;
}

/** How a Fix match query will be read, mirroring the server's
 * `parse_id_query`: an IMDb id, a prefixed id (`tvdb:60620`), a provider
 * page link, a bare number (looked up only with one provider chosen), or a
 * title (`null`). */
export type IdHint = 'imdb' | 'prefixed' | 'link' | 'number';

const ID_PREFIXES = new Set([
	'imdb',
	'omdb',
	'tmdb',
	'tvdb',
	'thetvdb',
	'anidb',
	'anilist',
	'anisearch',
	'imvdb',
	'vgmdb',
	'audiodb'
]);

export function idHint(query: string): IdHint | null {
	const value = query.trim().toLowerCase();
	if (!value || /\s/.test(value)) return null;
	if (/^tt\d{7,10}$/.test(value)) return 'imdb';
	if (/^\d{1,10}$/.test(value)) return 'number';
	const prefixed = /^([a-z]+):(tt\d{7,10}|\d{1,10})$/.exec(value);
	if (prefixed && ID_PREFIXES.has(prefixed[1])) return 'prefixed';
	if (
		/^(https?:\/\/)?(www\.)?(imdb\.com\/title\/tt\d|themoviedb\.org\/(movie|tv)\/\d|thetvdb\.com\/|anilist\.co\/anime\/\d|anidb\.net\/(anime\/|a)\d|anisearch\.(com|de)\/anime\/\d|vgmdb\.net\/album\/\d)/.test(
			value
		)
	)
		return 'link';
	return null;
}
