// Admin "Miniatury" page (Faza 6): `GET /api/admin/trickplay` and the small
// pure helpers its view uses.

import type { components } from '$lib/api/schema';
import { api } from '$lib/playback/api';

export type TrickplayStatus = components['schemas']['TrickplayStatus'];
export type LibraryTrickplayStats = components['schemas']['LibraryTrickplayStats'];
export type TrickplayFailure = components['schemas']['TrickplayFailure'];

/** How often the page asks for fresh numbers while it is visible. */
export const POLL_MS = 3000;

export function fetchTrickplayStatus(token: string): Promise<TrickplayStatus> {
	return api<TrickplayStatus>(token, '/api/admin/trickplay');
}

/** Throws the item's thumbnails away and queues it again (202). */
export function regenerateTrickplay(token: string, itemId: string): Promise<void> {
	return api<void>(token, `/api/admin/media/${encodeURIComponent(itemId)}/trickplay`, 'POST');
}

/** Done (ready + failed) out of everything, 0..100, rounded down so 100
 * means really finished. An empty catalogue counts as done. */
export function percentDone(ready: number, failed: number, total: number): number {
	if (total <= 0) return 100;
	return Math.min(100, Math.floor(((ready + failed) / total) * 100));
}

export type Eta =
	| { kind: 'soon' }
	| { kind: 'minutes'; minutes: number }
	| { kind: 'hours'; hours: number; minutes: number };

/** A rough remaining time: under a minute, minutes, or hours + minutes. */
export function eta(seconds: number): Eta {
	const minutes = Math.round(Math.max(0, seconds) / 60);
	if (minutes < 1) return { kind: 'soon' };
	if (minutes < 60) return { kind: 'minutes', minutes };
	return { kind: 'hours', hours: Math.floor(minutes / 60), minutes: minutes % 60 };
}

/** What the worker is doing, for the status chip. */
export type WorkerState = 'disabled' | 'no_ffmpeg' | 'paused' | 'working' | 'idle' | 'done';

export function workerState(status: TrickplayStatus): WorkerState {
	if (!status.enabled) return 'disabled';
	if (status.ffmpeg_missing) return 'no_ffmpeg';
	if (status.current) return 'working';
	if (status.paused_for_playback && status.pending > 0) return 'paused';
	return status.pending > 0 ? 'idle' : 'done';
}
