import { describe, it, expect, vi, afterEach } from 'vitest';
import {
	fetchLibraries,
	createLibrary,
	deleteLibrary,
	scanLibrary,
	moveLibrary
} from './libraries';
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

describe('fetchLibraries', () => {
	it('sends the auth header and returns the parsed list', async () => {
		const libraries = [
			{
				id: 'lib-1',
				name: 'Movies',
				library_type: 'movies',
				sort_order: 0,
				root_paths: ['/media/movies'],
				save_metadata_in_media_folder: true
			}
		];
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(libraries) });

		const result = await fetchLibraries('tok123');

		expect(result).toEqual(libraries);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
	});

	it('throws AdminApiError with the status on a non-ok response', async () => {
		stubFetch({ ok: false, status: 403 });

		await expect(fetchLibraries('tok123')).rejects.toBeInstanceOf(AdminApiError);
	});
});

describe('createLibrary', () => {
	it('POSTs name/library_type/root_paths as JSON and returns the created library', async () => {
		const created = {
			id: 'lib-2',
			name: 'Seriale',
			library_type: 'tv_shows',
			sort_order: 1,
			root_paths: ['/media/tv', '/media/tv2'],
			save_metadata_in_media_folder: false
		};
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(created) });

		const result = await createLibrary(
			'tok123',
			'Seriale',
			'tv_shows',
			['/media/tv', '/media/tv2'],
			false
		);

		expect(result).toEqual(created);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries');
		expect(options.method).toBe('POST');
		expect(JSON.parse(options.body)).toEqual({
			name: 'Seriale',
			library_type: 'tv_shows',
			root_paths: ['/media/tv', '/media/tv2'],
			save_metadata_in_media_folder: false
		});
	});

	it('defaults save_metadata_in_media_folder to true when the caller omits it', async () => {
		const created = {
			id: 'lib-3',
			name: 'Filmy',
			library_type: 'movies',
			sort_order: 2,
			root_paths: ['/media/movies'],
			save_metadata_in_media_folder: true
		};
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(created) });

		await createLibrary('tok123', 'Filmy', 'movies', ['/media/movies']);

		const [, options] = fetchMock.mock.calls[0];
		expect(JSON.parse(options.body)).toEqual({
			name: 'Filmy',
			library_type: 'movies',
			root_paths: ['/media/movies'],
			save_metadata_in_media_folder: true
		});
	});

	it('throws AdminApiError on a non-ok response', async () => {
		stubFetch({ ok: false, status: 400 });

		await expect(createLibrary('tok123', '', 'movies', [])).rejects.toBeInstanceOf(AdminApiError);
	});
});

describe('deleteLibrary', () => {
	it('DELETEs the library-id path, URL-encoded', async () => {
		const fetchMock = stubFetch({ ok: true });

		await deleteLibrary('tok123', 'lib id/x');

		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries/lib%20id%2Fx');
		expect(options.method).toBe('DELETE');
	});
});

describe('scanLibrary', () => {
	it('POSTs to the /scan sub-path and returns the parsed summary', async () => {
		const summary = {
			movies_found: 2,
			episodes_found: 0,
			unrecognized: 1,
			metadata_matched: 2,
			images_saved: 2
		};
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(summary) });

		const result = await scanLibrary('tok123', 'lib-1');

		expect(result).toEqual(summary);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries/lib-1/scan');
		expect(options.method).toBe('POST');
	});
});

describe('moveLibrary', () => {
	it('PUTs the direction as JSON to the /reorder sub-path, URL-encoded', async () => {
		const fetchMock = stubFetch({ ok: true });

		await moveLibrary('tok123', 'lib id/x', 'up');

		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries/lib%20id%2Fx/reorder');
		expect(options.method).toBe('PUT');
		expect(JSON.parse(options.body)).toEqual({ direction: 'up' });
	});

	it('sends "down" as-is for the other direction', async () => {
		const fetchMock = stubFetch({ ok: true });

		await moveLibrary('tok123', 'lib-1', 'down');

		const [, options] = fetchMock.mock.calls[0];
		expect(JSON.parse(options.body)).toEqual({ direction: 'down' });
	});

	it('throws AdminApiError on a non-ok response', async () => {
		stubFetch({ ok: false, status: 404 });

		await expect(moveLibrary('tok123', 'lib-1', 'up')).rejects.toBeInstanceOf(AdminApiError);
	});
});
