// Global search (Faza 11): `GET /api/search?q=` and the omnibox helpers —
// debounce timing, the keyboard shortcut, episode labels.

import type { components } from '$lib/api/schema';
import { buildAuthHeader } from '$lib/auth';
import type { MediaItem } from './api';

type Raw = components['schemas']['SearchResponse'];
export type EpisodeResult = components['schemas']['EpisodeSearchResult'];
export type PersonResult = components['schemas']['PersonSearchResult'];
export type SearchResults = Omit<Raw, 'movies' | 'series' | 'collections'> & {
	movies: MediaItem[];
	series: MediaItem[];
	collections: MediaItem[];
};

/** The server answers shorter queries with empty groups: not worth asking. */
export const MIN_QUERY_CHARS = 2;
/** Wait this long after the last keystroke before asking the server. */
export const DEBOUNCE_MS = 300;

export function searchCatalog(
	token: string,
	query: string,
	signal?: AbortSignal
): Promise<SearchResults> {
	const url = `/api/search?q=${encodeURIComponent(query.trim())}`;
	return fetch(url, {
		headers: buildAuthHeader(token),
		signal
	}).then(async (response) => {
		if (!response.ok) throw new Error(`GET /api/search failed with status ${response.status}`);
		return (await response.json()) as SearchResults;
	});
}

export function isSearchable(query: string): boolean {
	return query.trim().length >= MIN_QUERY_CHARS;
}

export function hasResults(results: SearchResults | null): boolean {
	return (
		!!results &&
		results.movies.length +
			results.series.length +
			results.episodes.length +
			results.people.length +
			results.collections.length >
			0
	);
}

/** "S2:E5 - Tytuł odcinka". */
export function episodeLabel(episode: Pick<EpisodeResult, 'season' | 'episode' | 'title'>): string {
	return `S${episode.season}:E${episode.episode} - ${episode.title}`;
}

/** The parts of a key event the shortcut looks at. */
export interface ShortcutKey {
	key: string;
	ctrlKey: boolean;
	metaKey: boolean;
	altKey: boolean;
	target: EventTarget | null;
}

/** Ctrl+K / ⌘K anywhere, `/` outside text fields: open the search. */
export function isSearchShortcut(event: ShortcutKey): boolean {
	if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === 'k')
		return true;
	if (event.key !== '/' || event.ctrlKey || event.metaKey || event.altKey) return false;
	const target = event.target as {
		tagName?: string;
		type?: string;
		isContentEditable?: boolean;
	} | null;
	const tag = target?.tagName?.toUpperCase();
	const typing =
		!!target &&
		(target.isContentEditable === true ||
			tag === 'TEXTAREA' ||
			tag === 'SELECT' ||
			(tag === 'INPUT' &&
				!['checkbox', 'radio', 'button', 'submit', 'range'].includes(target.type ?? 'text')));
	return !typing;
}
