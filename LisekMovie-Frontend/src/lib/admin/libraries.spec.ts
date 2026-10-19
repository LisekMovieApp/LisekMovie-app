import { describe, it, expect, vi, afterEach } from 'vitest';
import {
	fetchLibraries,
	createLibrary,
	deleteLibrary,
	scanLibrary,
	startScan,
	watchScanJob,
	moveLibrary,
	type ScanJob
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

function job(overrides: Partial<ScanJob> = {}): ScanJob {
	return {
		id: 'job-1',
		library_id: 'lib-1',
		status: 'queued',
		phase: 'queued',
		phase_done: 0,
		phase_total: 0,
		files_found: 0,
		movies_found: 0,
		episodes_found: 0,
		unrecognized: 0,
		probed: 0,
		probe_failed: 0,
		metadata_matched: 0,
		images_saved: 0,
		error: null,
		created_at: '2026-10-19T00:00:00Z',
		started_at: null,
		finished_at: null,
		...overrides
	};
}

describe('startScan', () => {
	it('POSTs to the /scan sub-path and returns the queued job', async () => {
		const fetchMock = stubFetch({ ok: true, status: 202, json: () => Promise.resolve(job()) });

		const result = await startScan('tok123', 'lib-1');

		expect(result.id).toBe('job-1');
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/libraries/lib-1/scan');
		expect(options.method).toBe('POST');
	});
});

describe('scanLibrary', () => {
	it('polls the job until it completes and reports every update', async () => {
		const responses = [
			job(),
			job({ status: 'running', phase: 'saving', phase_done: 1, phase_total: 2 }),
			job({
				status: 'completed',
				phase: 'done',
				movies_found: 2,
				metadata_matched: 2,
				images_saved: 2
			})
		];
		const fetchMock = vi
			.fn()
			.mockImplementation(() =>
				Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(responses.shift()) })
			);
		vi.stubGlobal('fetch', fetchMock);
		const seen: string[] = [];

		const result = await scanLibrary('tok123', 'lib-1', (j) => seen.push(j.phase), {
			pollMs: 1,
			createSocket: null
		});

		expect(result.movies_found).toBe(2);
		expect(seen).toEqual(['queued', 'saving', 'done']);
		expect(fetchMock.mock.calls[1][0]).toBe('/api/admin/scan-jobs/job-1');
	});

	it('rejects when the scan fails', async () => {
		const responses = [job(), job({ status: 'failed', error: 'library_not_found' })];
		vi.stubGlobal(
			'fetch',
			vi
				.fn()
				.mockImplementation(() =>
					Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve(responses.shift()) })
				)
		);

		await expect(
			scanLibrary('tok123', 'lib-1', () => {}, { pollMs: 1, createSocket: null })
		).rejects.toThrow('library_not_found');
	});
});

describe('watchScanJob', () => {
	it('applies scan_progress events from the socket for this job only', async () => {
		vi.stubGlobal('window', { location: { href: 'http://localhost:5173/admin/libraries' } });
		vi.stubGlobal(
			'fetch',
			vi.fn().mockImplementation(() => new Promise(() => {}))
		);
		const sent: string[] = [];
		const socket = {
			send: (data: string) => sent.push(data),
			close: vi.fn()
		} as unknown as WebSocket & { onopen: () => void; onmessage: (e: { data: string }) => void };
		let socketUrl = '';
		const updates: ScanJob[] = [];

		const watch = watchScanJob('tok123', job(), (j) => updates.push(j), {
			pollMs: 60_000,
			createSocket: (url) => {
				socketUrl = url;
				return socket;
			}
		});
		socket.onopen();
		socket.onmessage({ data: JSON.stringify({ type: 'library_changed', items: [] }) });
		socket.onmessage({
			data: JSON.stringify({ type: 'scan_progress', job: job({ id: 'other' }) })
		});
		socket.onmessage({
			data: JSON.stringify({
				type: 'scan_progress',
				job: job({ status: 'completed', phase: 'done' })
			})
		});

		const finished = await watch.done;
		expect(socketUrl).toBe('ws://localhost:5173/api/notifications/socket');
		expect(JSON.parse(sent[0])).toEqual({ token: 'tok123' });
		expect(updates.map((u) => u.id)).toEqual(['job-1']);
		expect(finished.status).toBe('completed');
		expect(socket.close).toHaveBeenCalled();
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
