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
