import { describe, it, expect, vi, afterEach } from 'vitest';
import {
	fetchLibraryItems,
	fetchMediaItem,
	updateMediaItem,
	fetchFallbackItems,
	migrateFallbackItem,
	fetchProviderOrder,
	setProviderOrder
} from './mediaItems';
import { AdminApiError } from './api';

function stubFetch(response: { ok: boolean; status?: number; json?: () => unknown }) {
	const fetchMock = vi.fn().mockResolvedValue({
		ok: response.ok,
		status: response.status ?? (response.ok ? 200 : 500),
		json: response.json ?? (() => Promise.resolve(undefined))
	});
	vi.stubGlobal('fetch', fetchMock);
	return fetchMock;
}

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('fetchLibraryItems', () => {
	it('GETs the library-scoped items endpoint and returns the parsed list', async () => {
		const items = [
			{
				id: 'item-1',
				item_type: 'movie',
				title: 'Dune',
				year: 2021,
				season: null,
				episode: null,
				metadata_locked: false,
				metadata_fallback: true
			}
		];
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(items) });

		const result = await fetchLibraryItems('tok123', 'lib-1');

		expect(result).toEqual(items);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries/lib-1/items');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
	});

	it('throws AdminApiError with the status on a non-ok response', async () => {
		stubFetch({ ok: false, status: 403 });
		await expect(fetchLibraryItems('tok123', 'lib-1')).rejects.toBeInstanceOf(AdminApiError);
	});
});

describe('fetchMediaItem', () => {
	it('GETs the one-item endpoint and returns the parsed detail', async () => {
		const detail = {
			id: 'item-1',
			library_id: 'lib-1',
			item_type: 'movie',
			title: 'Dune',
			original_title: 'Dune',
			year: 2021,
			overview: 'A boy...',
			season: null,
			episode: null,
			metadata_locked: false,
			forced_provider: null,
			locked_fields: []
		};
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(detail) });

		const result = await fetchMediaItem('tok123', 'item-1');

		expect(result).toEqual(detail);
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/media/item-1');
	});
});

describe('updateMediaItem', () => {
	it('PUTs the edited fields as JSON', async () => {
		const fetchMock = stubFetch({ ok: true });
		const payload = {
			title: 'Dune',
			original_title: 'Dune',
			year: 2021,
			overview: 'Edited overview',
			metadata_locked: true,
			forced_provider: 'tmdb',
			locked_fields: ['overview']
		};

		await updateMediaItem('tok123', 'item-1', payload);

		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/media/item-1');
		expect(options.method).toBe('PUT');
		expect(JSON.parse(options.body)).toEqual(payload);
	});

	it('throws AdminApiError with the status on a non-ok response', async () => {
		stubFetch({ ok: false, status: 404 });
		await expect(
			updateMediaItem('tok123', 'missing', {
				title: 'X',
				original_title: null,
				year: null,
				overview: null,
				metadata_locked: false,
				forced_provider: null,
				locked_fields: []
			})
		).rejects.toBeInstanceOf(AdminApiError);
	});
});

describe('fetchFallbackItems', () => {
	it('GETs the fallback list', async () => {
		const items = [
			{ id: 'item-1', library_id: 'lib-1', title: 'Dune', file_path: '/media/dune.mkv' }
		];
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(items) });

		const result = await fetchFallbackItems('tok123');

		expect(result).toEqual(items);
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/media/fallback');
	});
});

describe('migrateFallbackItem', () => {
	it('POSTs to the migrate endpoint and returns the parsed result', async () => {
		const fetchMock = stubFetch({
			ok: true,
			json: () => Promise.resolve({ poster_migrated: true, backdrop_migrated: false })
		});

		const result = await migrateFallbackItem('tok123', 'item-1');

		expect(result).toEqual({ poster_migrated: true, backdrop_migrated: false });
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/media/item-1/migrate-to-media-folder');
		expect(options.method).toBe('POST');
	});
});

describe('fetchProviderOrder', () => {
	it('GETs the library+item-type-scoped order and unwraps it', async () => {
		const fetchMock = stubFetch({
			ok: true,
			json: () => Promise.resolve({ provider_order: ['tmdb', 'thetvdb'] })
		});

		const result = await fetchProviderOrder('tok123', 'lib-1', 'movie');

		expect(result).toEqual(['tmdb', 'thetvdb']);
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/libraries/lib-1/provider-order/movie');
	});
});

describe('setProviderOrder', () => {
	it('PUTs the new order as JSON', async () => {
		const fetchMock = stubFetch({ ok: true });

		await setProviderOrder('tok123', 'lib-1', 'movie', ['thetvdb', 'tmdb']);

		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries/lib-1/provider-order/movie');
		expect(options.method).toBe('PUT');
		expect(JSON.parse(options.body)).toEqual({ provider_order: ['thetvdb', 'tmdb'] });
	});
});
