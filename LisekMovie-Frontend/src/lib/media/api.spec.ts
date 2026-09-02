import { describe, it, expect, vi, afterEach } from 'vitest';
import {
	fetchHomeFeed,
	fetchLibraries,
	fetchLibraryItems,
	LibraryNotFoundError,
	imageUrl
} from './api';

describe('imageUrl', () => {
	it('appends the token as a query param', () => {
		expect(imageUrl('/api/media/abc/poster', 'tok123')).toBe('/api/media/abc/poster?token=tok123');
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

	it('URL-encodes the library id and returns the parsed library+items', async () => {
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

		const result = await fetchLibraryItems('tok123', 'lib id/x');

		expect(result).toEqual(body);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/libraries/lib%20id%2Fx/items');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
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
