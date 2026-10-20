import { buildAuthHeader } from '$lib/auth';
import type { components } from '$lib/api/schema';
import type { PlaybackProgress, ProgressAck } from '$lib/player/telemetry';

export type PlaybackSession = components['schemas']['StartResponse'];
export type StartRequest = components['schemas']['StartRequest'];
export type AudioTrack = components['schemas']['AudioTrack'];
export type SubtitleTrack = components['schemas']['SubtitleTrack'];
export type DownmixPreference = components['schemas']['DownmixPreference'];
export type NextEpisode = components['schemas']['NextEpisodeResponse'];
export interface MediaState {
	position_seconds: number;
	duration_seconds: number;
	watched: boolean;
	favorite: boolean;
}
export interface ListEntry extends MediaState {
	id: string;
	title: string;
	item_type: string;
	season: number | null;
	episode: number | null;
}

/** A failed API call with the server's `error_code` when it sent one. */
export class ApiError extends Error {
	constructor(
		readonly status: number,
		readonly code: string | null
	) {
		super(`Request failed: ${status}${code ? ` (${code})` : ''}`);
	}
}

export async function api<T>(
	token: string,
	path: string,
	method = 'GET',
	body?: unknown,
	keepalive = method === 'PUT' || method === 'DELETE'
): Promise<T> {
	const response = await fetch(path, {
		method,
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		...(body === undefined ? {} : { body: JSON.stringify(body) }),
		keepalive
	});
	if (!response.ok) {
		const code = await response
			.json()
			.then((data: { error_code?: string }) => data.error_code ?? null)
			.catch(() => null);
		throw new ApiError(response.status, code);
	}
	return response.status === 204 ? (undefined as T) : ((await response.json()) as T);
}

/** Starts playback; the server decides direct play, remux or transcode from `request.client`. */
export function startPlayback(token: string, id: string, request: StartRequest = {}) {
	return api<PlaybackSession>(
		token,
		`/api/media/${encodeURIComponent(id)}/playback`,
		'POST',
		request
	);
}

/**
 * Sends a telemetry progress report. `final` reports (leaving the page)
 * use `keepalive` so the browser delivers them after the page is gone.
 */
export function reportProgress(token: string, report: PlaybackProgress, final = false) {
	return api<ProgressAck>(token, '/api/telemetry/playback-progress', 'POST', report, final);
}

/** Older per-session progress endpoint, kept for other callers. */
export function saveProgress(
	token: string,
	sessionId: string,
	position: number,
	duration: number,
	paused: boolean,
	ended = false
) {
	if (!Number.isFinite(position) || !Number.isFinite(duration) || duration <= 0)
		return Promise.resolve();
	return api<void>(token, `/api/playback/${encodeURIComponent(sessionId)}/progress`, 'PUT', {
		position_seconds: Math.max(0, Math.min(position, duration)),
		duration_seconds: duration,
		paused,
		ended
	});
}

export function stopPlayback(token: string, sessionId: string) {
	return api<void>(token, `/api/playback/${encodeURIComponent(sessionId)}`, 'DELETE');
}

export function fetchMyList(token: string) {
	return api<ListEntry[]>(token, '/api/me/list');
}

/** The episode after `id` in its series, or null (a movie, the last episode). */
export async function fetchNextEpisode(token: string, id: string): Promise<NextEpisode | null> {
	return (await api<NextEpisode | undefined>(token, `/api/media/${encodeURIComponent(id)}/next`)) ?? null;
}
