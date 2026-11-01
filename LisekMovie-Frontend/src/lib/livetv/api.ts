/**
 * Faza 20 — Live TV: channels from an M3U playlist, the programme guide
 * from XMLTV, live streams started on the server (FFmpeg → live HLS).
 */
import { buildAuthHeader } from '$lib/auth';

export interface LiveProgram {
	id: number;
	channel_id: string;
	starts_at: string;
	ends_at: string;
	title: string;
	subtitle: string | null;
	description: string | null;
	category: string | null;
}

export interface LiveChannel {
	id: string;
	library_id: string;
	name: string;
	number: string | null;
	group_title: string | null;
	has_logo: boolean;
	logo_url: string | null;
}

export interface ChannelNow extends LiveChannel {
	now: LiveProgram | null;
	next: LiveProgram | null;
}

export interface GuideChannel extends LiveChannel {
	programs: LiveProgram[];
}

export interface Guide {
	start: string;
	end: string;
	now: string;
	channels: GuideChannel[];
}

export interface LiveSession {
	session_id: string;
	stream_url: string;
	channel_id: string;
	name: string;
	transcoding: boolean;
}

export interface LiveTvSource {
	library_id: string;
	library_name: string;
	m3u_url: string;
	xmltv_url: string | null;
	refreshed_at: string | null;
	last_error: string | null;
	channel_count: number;
	program_count: number;
}

export class LiveTvError extends Error {
	constructor(
		public status: number,
		public code: string
	) {
		super(code);
	}
}

async function request<T>(token: string, path: string, init: RequestInit = {}): Promise<T> {
	const response = await fetch(path, {
		...init,
		cache: 'no-store',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' }
	});
	if (response.status === 204) return undefined as T;
	const body = await response.json().catch(() => null);
	if (!response.ok) throw new LiveTvError(response.status, body?.error_code ?? 'internal_error');
	return body as T;
}

export function fetchLiveStatus(token: string) {
	return request<{ available: boolean; channel_count: number }>(token, '/api/livetv/status');
}

export function fetchChannels(token: string) {
	return request<ChannelNow[]>(token, '/api/livetv/channels');
}

export function fetchGuide(token: string, start: Date, hours: number) {
	const query = new URLSearchParams({ start: start.toISOString(), hours: String(hours) });
	return request<Guide>(token, `/api/livetv/guide?${query}`);
}

export function playChannel(token: string, channelId: string, transcode = false) {
	return request<LiveSession>(token, `/api/livetv/channels/${encodeURIComponent(channelId)}/play`, {
		method: 'POST',
		body: JSON.stringify({ transcode })
	});
}

/** Stops a live stream; `keepalive` lets it run while the page unloads. */
export function stopLive(token: string, sessionId: string, keepalive = false) {
	return fetch(`/api/livetv/sessions/${encodeURIComponent(sessionId)}`, {
		method: 'DELETE',
		headers: buildAuthHeader(token),
		keepalive
	}).catch(() => undefined);
}

export function logoSrc(channel: Pick<LiveChannel, 'logo_url'>, token: string): string | null {
	return channel.logo_url ? `${channel.logo_url}?token=${encodeURIComponent(token)}` : null;
}

// ---- Admin -------------------------------------------------------------------

export function fetchLiveSources(token: string) {
	return request<LiveTvSource[]>(token, '/api/admin/livetv');
}

export function setLiveSource(
	token: string,
	libraryId: string,
	m3uUrl: string,
	xmltvUrl: string | null
) {
	return request<LiveTvSource>(token, `/api/admin/livetv/${encodeURIComponent(libraryId)}`, {
		method: 'PUT',
		body: JSON.stringify({ m3u_url: m3uUrl, xmltv_url: xmltvUrl })
	});
}

export function refreshLiveSource(token: string, libraryId: string) {
	return request<LiveTvSource>(
		token,
		`/api/admin/livetv/${encodeURIComponent(libraryId)}/refresh`,
		{
			method: 'POST'
		}
	);
}

// ---- Guide layout ------------------------------------------------------------

/** Pixels per minute on the guide's time axis. */
export const PX_PER_MINUTE = 4;

/** Where a programme sits on the axis starting at `start` (clipped to the window). */
export function programBox(
	program: Pick<LiveProgram, 'starts_at' | 'ends_at'>,
	start: Date,
	end: Date
): { left: number; width: number } | null {
	const from = Math.max(Date.parse(program.starts_at), start.getTime());
	const to = Math.min(Date.parse(program.ends_at), end.getTime());
	if (!(to > from)) return null;
	const left = ((from - start.getTime()) / 60000) * PX_PER_MINUTE;
	return { left, width: ((to - from) / 60000) * PX_PER_MINUTE };
}

/** The window's start: the half hour before `now`, minus `pastMinutes`. */
export function guideStart(now: Date, pastMinutes = 30): Date {
	const start = new Date(now.getTime() - pastMinutes * 60000);
	start.setMinutes(start.getMinutes() < 30 ? 0 : 30, 0, 0);
	return start;
}

/** Share (0–1) of a programme already shown at `now`. */
export function progressOf(
	program: Pick<LiveProgram, 'starts_at' | 'ends_at'>,
	now: number
): number {
	const start = Date.parse(program.starts_at);
	const end = Date.parse(program.ends_at);
	if (!(end > start)) return 0;
	return Math.min(1, Math.max(0, (now - start) / (end - start)));
}
