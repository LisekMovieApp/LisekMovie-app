import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	episodeLabel,
	hasResults,
	isSearchable,
	isSearchShortcut,
	searchCatalog,
	type SearchResults,
	type ShortcutKey
} from './search';

afterEach(() => vi.unstubAllGlobals());

const empty: SearchResults = {
	query: 'x',
	movies: [],
	series: [],
	episodes: [],
	people: [],
	collections: []
};

function key(init: Partial<ShortcutKey>, target: ShortcutKey['target'] = null): ShortcutKey {
	return { key: '', ctrlKey: false, metaKey: false, altKey: false, ...init, target };
}

const body = { tagName: 'BODY' } as unknown as EventTarget;
const field = { tagName: 'INPUT', type: 'search' } as unknown as EventTarget;

describe('omnibox', () => {
	it('searches from two characters', () => {
		expect(isSearchable(' a ')).toBe(false);
		expect(isSearchable('ab')).toBe(true);
	});

	it('labels episodes the VOD way', () => {
		expect(episodeLabel({ season: 2, episode: 5, title: 'Rzeka' })).toBe('S2:E5 - Rzeka');
	});

	it('knows when there is anything to show', () => {
		expect(hasResults(null)).toBe(false);
		expect(hasResults(empty)).toBe(false);
		expect(hasResults({ ...empty, people: [{ person_id: 'p', name: 'A', role: 'Actor' }] })).toBe(
			true
		);
	});

	it('opens on Ctrl+K anywhere and on / outside text fields', () => {
		expect(isSearchShortcut(key({ key: 'k', ctrlKey: true }))).toBe(true);
		expect(isSearchShortcut(key({ key: 'K', metaKey: true }))).toBe(true);
		expect(isSearchShortcut(key({ key: '/' }, body))).toBe(true);
		expect(isSearchShortcut(key({ key: '/' }, field))).toBe(false);
		expect(isSearchShortcut(key({ key: 'k' }))).toBe(false);
	});

	it('asks the server with the trimmed query and the session', async () => {
		const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
			void url;
			void init;
			return new Response(JSON.stringify(empty));
		});
		vi.stubGlobal('fetch', fetchMock);
		await searchCatalog('tok', '  za szybcy ');
		expect(fetchMock.mock.calls[0][0]).toBe('/api/search?q=za%20szybcy');
		expect(fetchMock.mock.calls[0][1]).toMatchObject({
			headers: { Authorization: expect.stringContaining('tok') }
		});
	});
});
