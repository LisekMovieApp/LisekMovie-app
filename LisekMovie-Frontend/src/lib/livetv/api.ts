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
	/** One of the server's own channels (Faza 26), not an IPTV one. */
	own?: boolean;
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

// ---- Own channels (Faza 26) ---------------------------------------------------------

export interface OwnNowNext {
	title: string;
	subtitle?: string | null;
	starts_at: string;
	ends_at: string;
}

export interface OwnChannel {
	id: string;
	number: string;
	name: string;
	group_title?: string | null;
	has_logo: boolean;
	now?: OwnNowNext | null;
	next?: OwnNowNext | null;
}

export interface OwnProgramme {
	title: string;
	subtitle?: string | null;
	description?: string | null;
	starts_at: string;
	ends_at: string;
}

const OWN_CHANNELS = '/api/tv/channels';
const OWN_GUIDE = '/api/tv/guide';
const OWN_ERRORS: Record<string, string> = {
	tv_capacity_reached: 'live_capacity_reached',
	tv_stream_unavailable: 'live_stream_unavailable'
};

/** Ids of the server's own channels seen so far (to pick the right play call). */
const ownIds = new Set<string>();

function program(channelId: string, p: OwnProgramme | OwnNowNext, index: number): LiveProgram {
	return {
		id: -(index + 1),
		channel_id: channelId,
		starts_at: p.starts_at,
		ends_at: p.ends_at,
		title: p.title,
		subtitle: p.subtitle ?? null,
		description: 'description' in p ? (p.description ?? null) : null,
		category: null
	};
}

/** An own channel in the shape the Live TV pages use. */
export function fromOwn(channel: OwnChannel): ChannelNow {
	return {
		id: channel.id,
		library_id: '',
		name: channel.name,
		number: channel.number,
		group_title: channel.group_title ?? null,
		has_logo: channel.has_logo,
		logo_url: channel.has_logo ? `${OWN_CHANNELS}/${encodeURIComponent(channel.id)}/logo` : null,
		own: true,
		now: channel.now ? program(channel.id, channel.now, 0) : null,
		next: channel.next ? program(channel.id, channel.next, 1) : null
	};
}

async function ownChannels(token: string): Promise<ChannelNow[]> {
	const list = (await request<OwnChannel[]>(token, OWN_CHANNELS)).map(fromOwn);
	list.forEach((c) => ownIds.add(c.id));
	return list;
}

/** Own channels first, then the IPTV ones; one failing source does not hide the other. */
export async function mergeSettled<T>(own: Promise<T[]>, live: Promise<T[]>): Promise<T[]> {
	const [a, b] = await Promise.allSettled([own, live]);
	if (a.status === 'rejected' && b.status === 'rejected') throw b.reason;
	return [...(a.status === 'fulfilled' ? a.value : []), ...(b.status === 'fulfilled' ? b.value : [])];
}

export async function fetchLiveStatus(token: string) {
	const [own, live] = await Promise.allSettled([
		request<OwnChannel[]>(token, OWN_CHANNELS),
		request<{ available: boolean; channel_count: number }>(token, '/api/livetv/status')
	]);
	if (own.status === 'rejected' && live.status === 'rejected') throw live.reason;
	const ownCount = own.status === 'fulfilled' ? own.value.length : 0;
	const liveStatus = live.status === 'fulfilled' ? live.value : { available: false, channel_count: 0 };
	return { available: liveStatus.available || ownCount > 0, channel_count: liveStatus.channel_count + ownCount };
}

export function fetchChannels(token: string) {
	return mergeSettled(ownChannels(token), request<ChannelNow[]>(token, '/api/livetv/channels'));
}

export async function fetchGuide(token: string, start: Date, hours: number): Promise<Guide> {
	const query = new URLSearchParams({ start: start.toISOString(), hours: String(hours) });
	const ownQuery = new URLSearchParams({ from: start.toISOString(), hours: String(Math.min(48, hours)) });
	const [own, live] = await Promise.allSettled([
		(async () => {
			const [channels, guide] = await Promise.all([
				ownChannels(token),
				request<{ channel_id: string; programmes: OwnProgramme[] }[]>(token, `${OWN_GUIDE}?${ownQuery}`)
			]);
			return channels.map(
				(channel): GuideChannel => ({
					...channel,
					programs: (guide.find((g) => g.channel_id === channel.id)?.programmes ?? []).map((p, i) =>
						program(channel.id, p, i)
					)
				})
			);
		})(),
		request<Guide>(token, `/api/livetv/guide?${query}`)
	]);
	if (own.status === 'rejected' && live.status === 'rejected') throw live.reason;
	const base: Guide =
		live.status === 'fulfilled'
			? live.value
			: {
					start: start.toISOString(),
					end: new Date(start.getTime() + hours * 3600_000).toISOString(),
					now: new Date().toISOString(),
					channels: []
				};
	return { ...base, channels: [...(own.status === 'fulfilled' ? own.value : []), ...base.channels] };
}

async function playOwn(token: string, channelId: string): Promise<LiveSession> {
	try {
		const started = await request<{ session_id: string; stream_url: string; channel_id: string; name: string }>(
			token,
			`${OWN_CHANNELS}/${encodeURIComponent(channelId)}/play`,
			{ method: 'POST' }
		);
		// Already encoded for the browser: a second try with transcoding would change nothing.
		return { ...started, transcoding: true };
	} catch (error) {
		if (error instanceof LiveTvError && OWN_ERRORS[error.code]) {
			throw new LiveTvError(error.status, OWN_ERRORS[error.code]);
		}
		throw error;
	}
}

function playIptv(token: string, channelId: string, transcode: boolean) {
	return request<LiveSession>(token, `/api/livetv/channels/${encodeURIComponent(channelId)}/play`, {
		method: 'POST',
		body: JSON.stringify({ transcode })
	});
}

/** Starts a channel: an own one, or an IPTV one (a 404 from the first guess tries the other). */
export async function playChannel(token: string, channelId: string, transcode = false) {
	const own = () => playOwn(token, channelId);
	const iptv = () => playIptv(token, channelId, transcode);
	const [first, second] = ownIds.has(channelId) ? [own, iptv] : [iptv, own];
	try {
		return await first();
	} catch (error) {
		if (error instanceof LiveTvError && error.status === 404) return second();
		throw error;
	}
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
