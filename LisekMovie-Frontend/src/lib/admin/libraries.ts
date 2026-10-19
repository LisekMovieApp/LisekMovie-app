// Client-side calls for the Admin Panel's libraries tab
// (`/api/admin/libraries/*` on the backend). Same split as `providers.ts`:
// pure request-shape logic here, unit-tested without rendering Svelte.

import { buildAuthHeader } from '$lib/auth';
import { assertOk } from './api';

export type LibraryType = 'movies' | 'tv_shows';

export interface Library {
	id: string;
	name: string;
	library_type: LibraryType;
	sort_order: number;
	root_paths: string[];
	/** Whether a metadata match's poster/backdrop are written directly next
	 * to the video file rather than only into the
	 * server's own local cache. Defaults to `true` server-side; see
	 * `AddLibraryModal`'s checkbox for where an admin can turn it off. */
	save_metadata_in_media_folder: boolean;
}

export interface ScanSummary {
	movies_found: number;
	episodes_found: number;
	unrecognized: number;
	/** How many scanned items matched an enabled, configured metadata
	 * provider this scan. `0` whenever no provider has both an API key set
	 * and its "Enabled" toggle on — see `images_saved`'s own comment. */
	metadata_matched: number;
	/** How many items actually got a poster and/or backdrop saved to disk
	 * this scan (media folder or local cache) — can be lower than
	 * `metadata_matched` when a match had no usable image. A library page
	 * showing `0` here alongside a nonzero `movies_found`/`episodes_found`
	 * is the signal to check the provider's API key, not a scanner bug. */
	images_saved: number;
}

export type ScanStatus = 'queued' | 'running' | 'completed' | 'failed';
export type ScanPhase = 'queued' | 'walking' | 'saving' | 'enriching' | 'probing' | 'done';

/** A background scan (`/api/admin/scan-jobs`). Carries every
 * `ScanSummary` field, filled in as the scan progresses. */
export interface ScanJob extends ScanSummary {
	id: string;
	library_id: string;
	status: ScanStatus;
	phase: ScanPhase;
	/** Progress within `phase`; `phase_total` is 0 while walking (the
	 * number of files is not known until the walk ends). */
	phase_done: number;
	phase_total: number;
	files_found: number;
	/** Files whose streams ffprobe read / could not read. */
	probed: number;
	probe_failed: number;
	error: string | null;
	created_at: string;
	started_at: string | null;
	finished_at: string | null;
}

export function isScanFinished(job: ScanJob): boolean {
	return job.status === 'completed' || job.status === 'failed';
}

export async function fetchLibraries(token: string): Promise<Library[]> {
	const response = await fetch('/api/admin/libraries', { headers: buildAuthHeader(token) });
	await assertOk(response);
	return (await response.json()) as Library[];
}

export async function createLibrary(
	token: string,
	name: string,
	libraryType: LibraryType,
	rootPaths: string[],
	saveMetadataInMediaFolder: boolean = true
): Promise<Library> {
	const response = await fetch('/api/admin/libraries', {
		method: 'POST',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({
			name,
			library_type: libraryType,
			root_paths: rootPaths,
			save_metadata_in_media_folder: saveMetadataInMediaFolder
		})
	});
	await assertOk(response);
	return (await response.json()) as Library;
}

export async function deleteLibrary(token: string, libraryId: string): Promise<void> {
	const response = await fetch(`/api/admin/libraries/${encodeURIComponent(libraryId)}`, {
		method: 'DELETE',
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
}

/** Queues a scan; the server answers 202 with the job right away (or the
 * scan already pending for this library). */
export async function startScan(token: string, libraryId: string): Promise<ScanJob> {
	const response = await fetch(`/api/admin/libraries/${encodeURIComponent(libraryId)}/scan`, {
		method: 'POST',
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
	return (await response.json()) as ScanJob;
}

export async function fetchScanJob(token: string, jobId: string): Promise<ScanJob> {
	const response = await fetch(`/api/admin/scan-jobs/${encodeURIComponent(jobId)}`, {
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
	return (await response.json()) as ScanJob;
}

/** The most recent scans across all libraries, newest first. */
export async function fetchScanJobs(token: string): Promise<ScanJob[]> {
	const response = await fetch('/api/admin/scan-jobs', { headers: buildAuthHeader(token) });
	await assertOk(response);
	return (await response.json()) as ScanJob[];
}

export interface WatchOptions {
	/** Fallback poll interval. Live updates come over the WebSocket; the
	 * poll covers a socket that is unavailable or missed an event. */
	pollMs?: number;
	/** Injection point for tests; `null` disables the socket. */
	createSocket?: ((url: string) => WebSocket) | null;
}

export interface ScanWatch {
	/** Resolves with the finished job (completed or failed). */
	done: Promise<ScanJob>;
	stop(): void;
}

/**
 * Follows a scan until it finishes: `scan_progress` events from
 * `/api/notifications/socket` (admin sockets only) for live progress, plus a
 * slow poll of `GET /api/admin/scan-jobs/{id}` as a fallback.
 */
export function watchScanJob(
	token: string,
	job: ScanJob,
	onUpdate: (job: ScanJob) => void,
	options: WatchOptions = {}
): ScanWatch {
	const pollMs = options.pollMs ?? 3000;
	const createSocket =
		options.createSocket === undefined ? defaultSocketFactory() : options.createSocket;
	let stopped = false;
	let timer: ReturnType<typeof setTimeout> | undefined;
	let socket: WebSocket | undefined;
	let latest = job;
	let finish: (job: ScanJob) => void = () => {};
	const done = new Promise<ScanJob>((resolve) => (finish = resolve));

	function stop() {
		stopped = true;
		clearTimeout(timer);
		socket?.close();
	}

	function update(next: ScanJob) {
		if (stopped || next.id !== job.id) return;
		latest = next;
		onUpdate(next);
		if (isScanFinished(next)) {
			stop();
			finish(next);
		}
	}

	async function poll() {
		if (stopped) return;
		try {
			update(await fetchScanJob(token, job.id));
		} catch {
			// Transient; the next poll retries.
		}
		if (!stopped) timer = setTimeout(poll, pollMs);
	}

	if (createSocket && typeof window !== 'undefined') {
		try {
			const url = new URL('/api/notifications/socket', window.location.href);
			url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
			socket = createSocket(url.toString());
			const current = socket;
			current.onopen = () => current.send(JSON.stringify({ token }));
			current.onmessage = (event) => {
				try {
					const data = JSON.parse(String(event.data));
					if (data?.type === 'scan_progress' && data.job) update(data.job as ScanJob);
				} catch {
					// Ignore malformed frames.
				}
			};
		} catch {
			socket = undefined;
		}
	}

	if (isScanFinished(latest)) {
		update(latest);
	} else {
		timer = setTimeout(poll, pollMs);
	}
	return { done, stop };
}

function defaultSocketFactory(): ((url: string) => WebSocket) | null {
	return typeof WebSocket === 'undefined' ? null : (url) => new WebSocket(url);
}

/** Queues a scan and resolves with its summary once it finishes;
 * `onProgress` receives every update in between. Rejects when the scan
 * fails. */
export async function scanLibrary(
	token: string,
	libraryId: string,
	onProgress: (job: ScanJob) => void = () => {},
	options: WatchOptions = {}
): Promise<ScanJob> {
	const job = await startScan(token, libraryId);
	onProgress(job);
	const finished = await watchScanJob(token, job, onProgress, options).done;
	if (finished.status === 'failed') throw new Error(finished.error ?? 'scan_failed');
	return finished;
}

/** Nudges one library up or down in the shared `sort_order` — the "W
 * górę"/"W dół" buttons on the libraries tab (Runda 9). Swaps with the
 * immediate neighbor server-side (see `lisek_db::move_library`'s doc
 * comment); the caller re-fetches the list afterwards rather than trying to
 * reorder its local copy, so it can never drift from what the backend
 * actually did. */
export async function moveLibrary(
	token: string,
	libraryId: string,
	direction: 'up' | 'down'
): Promise<void> {
	const response = await fetch(`/api/admin/libraries/${encodeURIComponent(libraryId)}/reorder`, {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ direction })
	});
	await assertOk(response);
}
