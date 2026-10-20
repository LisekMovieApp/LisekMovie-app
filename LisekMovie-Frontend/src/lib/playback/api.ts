import { buildAuthHeader } from '$lib/auth';
import type { components } from '$lib/api/schema';

export type PlaybackSession = components['schemas']['StartResponse'];
export type StartRequest = components['schemas']['StartRequest'];
export type AudioTrack = components['schemas']['AudioTrack'];
export type SubtitleTrack = components['schemas']['SubtitleTrack'];
export type DownmixPreference = components['schemas']['DownmixPreference'];
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
}
export async function api<T>(token: string, path: string, method = 'GET', body?: unknown): Promise<T> {
    const response = await fetch(path, {
        method,
        headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        keepalive: method === 'PUT' || method === 'DELETE'
    });
    if (!response.ok) throw new Error(`Request failed: ${response.status}`);
    return response.status === 204 ? undefined as T : await response.json() as T;
}
/** Starts playback; the server decides direct play, remux or transcode from `request.client`. */
export function startPlayback(token: string, id: string, request: StartRequest = {}) {
    return api<PlaybackSession>(token, `/api/media/${encodeURIComponent(id)}/playback`, 'POST', request);
}
export function saveProgress(token: string, sessionId: string, position: number, duration: number, paused: boolean, ended = false) {
    if (!Number.isFinite(position) || !Number.isFinite(duration) || duration <= 0) return Promise.resolve();
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
