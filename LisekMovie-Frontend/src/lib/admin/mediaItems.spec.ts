import { describe, it, expect, vi, afterEach } from 'vitest';
import {
	fetchLibraryItems,
	fetchMediaItem,
	updateMediaItem,
	fetchFallbackItems,
	migrateFallbackItem,
	fetchProviderOrder,
	setProviderOrder,
	fetchSeason,
	updateSeason,
	setSeasonPoster,
	parseNames,
	parseRating,
	normalizeIds,
	refreshMetadata,
	sameIds,
	validProviderId
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

describe('season editor calls (Faza 21)', () => {
	it('loads and saves a season through any of its episodes', async () => {
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve({ season: 2 }) });
		await fetchSeason('tok', 'ep-1', 2);
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/media/ep-1/seasons/2');
		await updateSeason('tok', 'ep-1', 2, {
			name: 'Sezon 2',
			overview: null,
			air_year: 2024,
			locked_fields: ['name']
		});
		const [url, init] = fetchMock.mock.calls[1];
		expect(url).toBe('/api/admin/media/ep-1/seasons/2');
		expect(init.method).toBe('PUT');
		expect(JSON.parse(init.body)).toEqual({
			name: 'Sezon 2',
			overview: null,
			air_year: 2024,
			locked_fields: ['name']
		});
	});

	it('sends a season poster as a file or a link', async () => {
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve({ url: '/p?v=1' }) });
		const file = new Blob(['x'], { type: 'image/png' });
		expect(await setSeasonPoster('tok', 'ep-1', 1, file)).toBe('/p?v=1');
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/media/ep-1/seasons/1/poster');
		expect(fetchMock.mock.calls[0][1].headers['Content-Type']).toBe('image/png');
		await setSeasonPoster('tok', 'ep-1', 1, 'https://example.org/p.jpg');
		expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toEqual({ url: 'https://example.org/p.jpg' });
	});
});

describe('score and directors fields (Faza 21)', () => {
	it('reads a score with a comma or a dot, rounded to one decimal', () => {
		expect(parseRating('7,46')).toBe(7.5);
		expect(parseRating(' 8 ')).toBe(8);
		expect(parseRating('')).toBeNull();
		expect(parseRating('11')).toBeUndefined();
		expect(parseRating('dużo')).toBeUndefined();
		expect(parseRating('-1')).toBeUndefined();
	});

	it('splits directors by line or comma, without duplicates', () => {
		expect(parseNames('Jan Reżyser\nAnna Kamera, jan reżyser\n \n')).toEqual([
			'Jan Reżyser',
			'Anna Kamera'
		]);
		expect(parseNames('')).toEqual([]);
	});
});

describe('external ids (Faza 22)', () => {
	it('checks ids per service', () => {
		expect(validProviderId('imdb', 'tt0133093')).toBe(true);
		expect(validProviderId('imdb', '0133093')).toBe(false);
		expect(validProviderId('tmdb', ' 603 ')).toBe(true);
		expect(validProviderId('tmdb', '0')).toBe(false);
		expect(validProviderId('tvdb', '12a')).toBe(false);
		expect(validProviderId('netflix', '1')).toBe(false);
	});

	it('compares ids the way the server stores them', () => {
		expect(normalizeIds({ tmdb: ' 603 ', imdb: 'TT0133093', tvdb: ' ' })).toEqual({
			imdb: 'tt0133093',
			tmdb: '603'
		});
		expect(sameIds({ tmdb: '603', imdb: 'tt1' }, { imdb: 'TT1', tmdb: '603 ' })).toBe(true);
		expect(sameIds({ tmdb: '603' }, { tmdb: '604' })).toBe(false);
	});

	it('fetches an item again', async () => {
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve({ items: 3, matched: 3 }) });
		expect(await refreshMetadata('tok', 'ep-1')).toEqual({ items: 3, matched: 3 });
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/media/ep-1/refresh');
		expect(fetchMock.mock.calls[0][1].method).toBe('POST');
	});
});
