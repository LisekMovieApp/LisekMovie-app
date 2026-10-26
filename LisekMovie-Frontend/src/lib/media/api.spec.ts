import { describe, it, expect, vi, afterEach } from 'vitest';
import {
	fetchHomeFeed,
	fetchLibraries,
	fetchLibraryItems,
	fetchLibraryRecent,
	fetchLibraryRecommended,
	fetchLibraryPopular,
	LibraryNotFoundError,
	imageUrl,
	fetchItemCollection,
	fetchCollection,
	MediaNotFoundError,
	yearSpan,
	ageLabel
} from './api';

describe('imageUrl', () => {
	it('appends the token as a query param', () => {
		expect(imageUrl('/api/media/abc/poster', 'tok123')).toBe('/api/media/abc/poster?token=tok123');
		expect(imageUrl('/api/media/abc/chapters/1/image?v=4000', 'tok123')).toBe(
			'/api/media/abc/chapters/1/image?v=4000&token=tok123'
		);
	});

	it('encodes characters a token could contain', () => {
		expect(imageUrl('/api/media/abc/poster', 'a+b/c')).toBe(
			'/api/media/abc/poster?token=a%2Bb%2Fc'
		);
	});
});

describe('fetchHomeFeed', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('sends the auth header and returns the parsed feed', async () => {
		const feed = {
			hero: null,
			recently_added: [],
			top10: [],
			recommended: [],
			popular: []
		};
		const fetchMock = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			json: () => Promise.resolve(feed)
		});
		vi.stubGlobal('fetch', fetchMock);

		const result = await fetchHomeFeed('tok123');

		expect(result).toEqual(feed);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/home');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
	});

	it('throws on a non-ok response', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn().mockResolvedValue({ ok: false, status: 401, json: () => Promise.resolve({}) })
		);

		await expect(fetchHomeFeed('tok123')).rejects.toThrow();
	});
});

describe('fetchLibraries', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('sends the auth header and returns the parsed list', async () => {
		const libraries = [{ id: 'lib-1', name: 'Filmy', library_type: 'movies' }];
		const fetchMock = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			json: () => Promise.resolve(libraries)
		});
		vi.stubGlobal('fetch', fetchMock);

		const result = await fetchLibraries('tok123');

		expect(result).toEqual(libraries);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/libraries');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
	});

	it('throws on a non-ok response', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn().mockResolvedValue({ ok: false, status: 500, json: () => Promise.resolve({}) })
		);

		await expect(fetchLibraries('tok123')).rejects.toThrow();
	});
});

describe('fetchLibraryItems', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('URL-encodes the library id and returns the parsed page (no options -> no query string)', async () => {
		const body = {
			library: { id: 'lib-1', name: 'Filmy', library_type: 'movies' },
			items: [],
			total: 0,
			page: 1,
			page_size: 100
		};
		const fetchMock = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			json: () => Promise.resolve(body)
		});
		vi.stubGlobal('fetch', fetchMock);

		const result = await fetchLibraryItems('tok123', 'lib id/x');

		expect(result).toEqual(body);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/libraries/lib%20id%2Fx/items');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
	});

	it('builds a query string from page/sort/letter when given', async () => {
		const fetchMock = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			json: () =>
				Promise.resolve({
					library: { id: 'lib-1', name: 'Filmy', library_type: 'movies' },
					items: [],
					total: 0,
					page: 2,
					page_size: 100
				})
		});
		vi.stubGlobal('fetch', fetchMock);

		await fetchLibraryItems('tok123', 'lib-1', { page: 2, sort: 'title_desc', letter: 'a' });

		const [url] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/libraries/lib-1/items?page=2&sort=title_desc&letter=a');
	});

	it('throws LibraryNotFoundError specifically for a 404', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn().mockResolvedValue({ ok: false, status: 404, json: () => Promise.resolve({}) })
		);

		await expect(fetchLibraryItems('tok123', 'lib-1')).rejects.toBeInstanceOf(LibraryNotFoundError);
	});

	it('throws a plain Error for any other non-ok response', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn().mockResolvedValue({ ok: false, status: 500, json: () => Promise.resolve({}) })
		);

		await expect(fetchLibraryItems('tok123', 'lib-1')).rejects.not.toBeInstanceOf(
			LibraryNotFoundError
		);
	});
});

describe('fetchLibraryRecent / fetchLibraryRecommended / fetchLibraryPopular', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	const cases: [
		name: string,
		fn: typeof fetchLibraryRecent,
		segment: 'recent' | 'recommended' | 'popular'
	][] = [
		['fetchLibraryRecent', fetchLibraryRecent, 'recent'],
		['fetchLibraryRecommended', fetchLibraryRecommended, 'recommended'],
		['fetchLibraryPopular', fetchLibraryPopular, 'popular']
	];

	for (const [name, fn, segment] of cases) {
		it(`${name} URL-encodes the library id and hits /${segment}`, async () => {
			const body = {
				library: { id: 'lib-1', name: 'Filmy', library_type: 'movies' },
				items: []
			};
			const fetchMock = vi.fn().mockResolvedValue({
				ok: true,
				status: 200,
				json: () => Promise.resolve(body)
			});
			vi.stubGlobal('fetch', fetchMock);

			const result = await fn('tok123', 'lib id/x');

			expect(result).toEqual(body);
			const [url, options] = fetchMock.mock.calls[0];
			expect(url).toBe(`/api/libraries/lib%20id%2Fx/${segment}`);
			expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
		});

		it(`${name} throws LibraryNotFoundError specifically for a 404`, async () => {
			vi.stubGlobal(
				'fetch',
				vi.fn().mockResolvedValue({ ok: false, status: 404, json: () => Promise.resolve({}) })
			);

			await expect(fn('tok123', 'lib-1')).rejects.toBeInstanceOf(LibraryNotFoundError);
		});

		it(`${name} throws a plain Error for any other non-ok response`, async () => {
			vi.stubGlobal(
				'fetch',
				vi.fn().mockResolvedValue({ ok: false, status: 500, json: () => Promise.resolve({}) })
			);

			await expect(fn('tok123', 'lib-1')).rejects.not.toBeInstanceOf(LibraryNotFoundError);
		});
	}
});

describe('collections (Faza 6)', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('asks for grouped collections only when requested', async () => {
		const fetchMock = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			json: () => Promise.resolve({ library: {}, items: [], total: 0, page: 1, page_size: 100 })
		});
		vi.stubGlobal('fetch', fetchMock);
		await fetchLibraryItems('t', 'lib', { groupCollections: true });
		await fetchLibraryItems('t', 'lib', { groupCollections: false });
		expect(fetchMock.mock.calls[0][0]).toBe('/api/libraries/lib/items?group_collections=true');
		expect(fetchMock.mock.calls[1][0]).toBe('/api/libraries/lib/items');
	});

	it('treats 204 as "not in a collection" and 404 as not found', async () => {
		vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, status: 204 }));
		expect(await fetchItemCollection('t', 'm')).toBeNull();
		vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, status: 404 }));
		await expect(fetchCollection('t', 'c')).rejects.toBeInstanceOf(MediaNotFoundError);
	});

	it('formats year spans and age badges', () => {
		expect(yearSpan(1977, 1983)).toBe('1977–1983');
		expect(yearSpan(2009, 2009)).toBe('2009');
		expect(yearSpan(null, null)).toBe('');
		expect(ageLabel({ age_rating: 13, certification: 'PG-13' })).toBe('PG-13');
		expect(ageLabel({ age_rating: 7, certification: null })).toBe('7+');
		expect(ageLabel({ age_rating: 0 })).toBe('0+');
		expect(ageLabel({ age_rating: null, certification: null })).toBeNull();
	});
});
