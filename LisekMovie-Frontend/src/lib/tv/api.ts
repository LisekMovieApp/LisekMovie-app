/**
 * Faza 26 — own TV channels: the administrator's API (channels, schedule,
 * fillers, IPTV channels) and the small pieces of logic the pages share
 * (weekday masks, slot defaults, labels, durations).
 */
import { buildAuthHeader } from '$lib/auth';
import type { components } from '$lib/api/schema';
import { AdminApiError, assertOk } from '$lib/admin/api';

type Schemas = components['schemas'];
export type AdminChannel = Schemas['AdminChannel'];
export type ChannelBody = Schemas['ChannelBody'];
export type Slot = Schemas['Slot'];
export type SlotInput = Schemas['SlotInput'];
export type SourceHit = Schemas['SourceHit'];
export type FillerFolder = Schemas['FillerFolder'];
export type FillerFile = Schemas['FillerFileRow'];
export type FillerPreset = Schemas['FillerPresetRow'];
export type PlayoutEntry = Schemas['PlayoutEntry'];
export type RebuildResult = Schemas['RebuildResult'];
export type AdminLiveChannel = Schemas['AdminLiveChannel'];
export type ChannelSettings = Schemas['ChannelSettings'];

export const ENDPOINTS = {
	channels: '/api/admin/tv/channels',
	order: '/api/admin/tv/channels/order',
	sources: '/api/admin/tv/sources',
	folders: '/api/admin/tv/fillers/folders',
	presets: '/api/admin/tv/fillers/presets',
	liveSources: '/api/admin/livetv',
	viewerChannels: '/api/tv/channels',
	viewerGuide: '/api/tv/guide'
} as const;

export const QUALITIES = ['480p', '720p', '1080p'] as const;
export const SLOT_MODES = ['one', 'multiple', 'duration', 'flood'] as const;
export const PLAY_ORDERS = ['chronological', 'season_episode', 'shuffle', 'random'] as const;
export const TAIL_MODES = ['offline', 'filler', 'overrun'] as const;
export const START_MODES = ['strict', 'flexible'] as const;
export const SOURCE_KINDS = ['library', 'series', 'movie', 'collection'] as const;
export const FILLER_KINDS = ['pre', 'post', 'tail', 'fallback'] as const;
export const FILLER_MODES = ['count', 'duration', 'pad'] as const;
export const MAX_SLOTS = 200;

/** Monday = bit 0 … Sunday = bit 6, as the server stores them. */
export const WEEKDAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'] as const;
export const ALL_DAYS = 127;

export function dayOn(mask: number, day: number): boolean {
	return (mask & (1 << day)) !== 0;
}

export function toggleDay(mask: number, day: number): number {
	return mask ^ (1 << day);
}

/** The days as a short key: all, weekdays, weekend, or null for a custom mix. */
export function daysPreset(mask: number): 'all' | 'weekdays' | 'weekend' | null {
	if (mask === ALL_DAYS) return 'all';
	if (mask === 0b0011111) return 'weekdays';
	if (mask === 0b1100000) return 'weekend';
	return null;
}

export async function request<T>(
	token: string,
	path: string,
	method = 'GET',
	body?: unknown
): Promise<T> {
	const response = await fetch(path, {
		method,
		cache: 'no-store',
		headers: {
			...buildAuthHeader(token),
			...(body === undefined ? {} : { 'Content-Type': 'application/json' })
		},
		body: body === undefined ? undefined : JSON.stringify(body)
	});
	await assertOk(response);
	if (response.status === 204) return undefined as T;
	return (await response.json()) as T;
}

export const errorCode = (error: unknown): string | null =>
	error instanceof AdminApiError ? error.code : null;

// ---- Channels ----------------------------------------------------------------------

export const fetchChannels = (token: string) => request<AdminChannel[]>(token, ENDPOINTS.channels);

export const createChannel = (token: string, body: ChannelBody) =>
	request<AdminChannel>(token, ENDPOINTS.channels, 'POST', body);

export const updateChannel = (token: string, id: string, body: ChannelBody) =>
	request<AdminChannel>(token, `${ENDPOINTS.channels}/${encodeURIComponent(id)}`, 'PUT', body);

export const deleteChannel = (token: string, id: string) =>
	request<void>(token, `${ENDPOINTS.channels}/${encodeURIComponent(id)}`, 'DELETE');

export const reorderChannels = (token: string, ids: string[]) =>
	request<void>(token, ENDPOINTS.order, 'PUT', { ids });

export async function uploadLogo(token: string, id: string, file: Blob): Promise<void> {
	const response = await fetch(`${ENDPOINTS.channels}/${encodeURIComponent(id)}/logo`, {
		method: 'POST',
		headers: { ...buildAuthHeader(token), 'Content-Type': file.type || 'application/octet-stream' },
		body: file
	});
	await assertOk(response);
}

export const deleteLogo = (token: string, id: string) =>
	request<void>(token, `${ENDPOINTS.channels}/${encodeURIComponent(id)}/logo`, 'DELETE');

/** A channel's logo for `<img>` (the session token in the query, like other images). */
export const logoUrl = (id: string, token: string, version = 0) =>
	`${ENDPOINTS.viewerChannels}/${encodeURIComponent(id)}/logo?token=${encodeURIComponent(token)}&v=${version}`;

// ---- Schedule ----------------------------------------------------------------------

export const fetchSlots = (token: string, id: string) =>
	request<Slot[]>(token, `${ENDPOINTS.channels}/${encodeURIComponent(id)}/slots`);

export const saveSlots = (token: string, id: string, slots: SlotInput[]) =>
	request<Slot[]>(token, `${ENDPOINTS.channels}/${encodeURIComponent(id)}/slots`, 'PUT', slots);

export const copySchedule = (token: string, id: string, from: string) =>
	request<Slot[]>(
		token,
		`${ENDPOINTS.channels}/${encodeURIComponent(id)}/copy-schedule`,
		'POST',
		{ from_channel_id: from }
	);

export const searchSources = (token: string, query: string) =>
	request<SourceHit[]>(token, `${ENDPOINTS.sources}?q=${encodeURIComponent(query)}`);

export const rebuildChannel = (token: string, id: string, mode: 'continue' | 'refresh' | 'reset') =>
	request<RebuildResult>(token, `${ENDPOINTS.channels}/${encodeURIComponent(id)}/rebuild`, 'POST', {
		mode
	});

export const previewChannel = (token: string, id: string, hours: number) =>
	request<PlayoutEntry[]>(
		token,
		`${ENDPOINTS.channels}/${encodeURIComponent(id)}/preview?hours=${hours}`
	);

/** A new slot for a found source: plays it in order, one item at a time. */
export function newSlot(source: Pick<SourceHit, 'id' | 'kind'>): SlotInput {
	const series = source.kind === 'series' || source.kind === 'collection';
	return {
		start_time: null,
		start_mode: 'strict',
		days_of_week: ALL_DAYS,
		source_kind: source.kind,
		source_id: source.id,
		play_order: series ? 'season_episode' : 'shuffle',
		mode: source.kind === 'movie' ? 'one' : 'multiple',
		count: source.kind === 'movie' ? null : 1,
		duration_minutes: null,
		tail_mode: 'offline',
		pre_filler_id: null,
		mid_filler_id: null,
		post_filler_id: null,
		tail_filler_id: null,
		title_override: null,
		in_guide: true
	};
}

/** A slot as the editor sends it back (the server's extras dropped). */
export function toInput(slot: Slot | SlotInput): SlotInput {
	const { source_kind, source_id } = slot;
	return {
		start_time: slot.start_time ?? null,
		start_mode: slot.start_mode,
		days_of_week: slot.days_of_week,
		source_kind,
		source_id,
		play_order: slot.play_order,
		mode: slot.mode,
		count: slot.count ?? null,
		duration_minutes: slot.duration_minutes ?? null,
		tail_mode: slot.tail_mode,
		pre_filler_id: slot.pre_filler_id ?? null,
		mid_filler_id: slot.mid_filler_id ?? null,
		post_filler_id: slot.post_filler_id ?? null,
		tail_filler_id: slot.tail_filler_id ?? null,
		title_override: slot.title_override ?? null,
		in_guide: slot.in_guide
	};
}

/** Why a slot cannot be saved yet (a message key), or null. */
export function slotProblem(slot: SlotInput): string | null {
	if (slot.start_time && !/^([01]\d|2[0-3]):[0-5]\d$/.test(slot.start_time))
		return 'tv_admin.problem_time';
	if (slot.days_of_week < 1 || slot.days_of_week > ALL_DAYS) return 'tv_admin.problem_days';
	if (slot.mode === 'multiple' && !(slot.count && slot.count >= 1 && slot.count <= 100))
		return 'tv_admin.problem_count';
	if (
		slot.mode === 'duration' &&
		!(slot.duration_minutes && slot.duration_minutes >= 1 && slot.duration_minutes <= 1440)
	)
		return 'tv_admin.problem_duration';
	return null;
}

/** Fields that do not apply to a mode are cleared (the server does the same). */
export function normalizeMode(slot: SlotInput): SlotInput {
	if (slot.mode === 'multiple') return { ...slot, duration_minutes: null, count: slot.count ?? 1 };
	if (slot.mode === 'duration') return { ...slot, count: null, duration_minutes: slot.duration_minutes ?? 60 };
	return { ...slot, count: null, duration_minutes: null };
}

export function moveItem<T>(list: T[], from: number, to: number): T[] {
	if (from === to || from < 0 || to < 0 || from >= list.length || to >= list.length) return list;
	const copy = [...list];
	const [item] = copy.splice(from, 1);
	copy.splice(to, 0, item);
	return copy;
}

/** The browser's time zone, when it is one the server knows. */
export function browserTimezone(): string {
	try {
		return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
	} catch {
		return 'UTC';
	}
}

export function timezones(): string[] {
	try {
		const list = (Intl as unknown as { supportedValuesOf?: (k: string) => string[] }).supportedValuesOf?.(
			'timeZone'
		);
		if (list?.length) return list.includes('UTC') ? list : ['UTC', ...list];
	} catch {
		// An older browser: the common ones below.
	}
	return ['UTC', 'Europe/Warsaw', 'Europe/London', 'Europe/Berlin', 'America/New_York'];
}

/** `1:05:30` / `5:30` from milliseconds. */
export function formatLength(ms: number): string {
	const total = Math.max(0, Math.round(ms / 1000));
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	const two = (n: number) => String(n).padStart(2, '0');
	return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** What a preview entry shows on its row: kind badge key + length. */
export const entryLength = (e: Pick<PlayoutEntry, 'starts_at' | 'ends_at'>) =>
	Date.parse(e.ends_at) - Date.parse(e.starts_at);

// ---- Fillers -----------------------------------------------------------------------

export const fetchFolders = (token: string) => request<FillerFolder[]>(token, ENDPOINTS.folders);

export const addFolder = (token: string, path: string) =>
	request<FillerFolder>(token, ENDPOINTS.folders, 'POST', { path });

export const setFolderEnabled = (token: string, id: string, enabled: boolean) =>
	request<void>(token, `${ENDPOINTS.folders}/${encodeURIComponent(id)}`, 'PUT', { enabled });

export const deleteFolder = (token: string, id: string) =>
	request<void>(token, `${ENDPOINTS.folders}/${encodeURIComponent(id)}`, 'DELETE');

export const scanFolder = (token: string, id: string) =>
	request<void>(token, `${ENDPOINTS.folders}/${encodeURIComponent(id)}/scan`, 'POST');

export const fetchFolderFiles = (token: string, id: string) =>
	request<FillerFile[]>(token, `${ENDPOINTS.folders}/${encodeURIComponent(id)}/files`);

export const fetchPresets = (token: string) => request<FillerPreset[]>(token, ENDPOINTS.presets);

export const savePreset = (token: string, preset: FillerPreset) =>
	request<FillerPreset>(token, ENDPOINTS.presets, 'POST', preset);

export const deletePreset = (token: string, id: string) =>
	request<void>(token, `${ENDPOINTS.presets}/${encodeURIComponent(id)}`, 'DELETE');

/** A preset as the form starts: after the content, one file. */
export function blankPreset(): FillerPreset {
	return {
		name: '',
		kind: 'post',
		mode: 'count',
		count: 1,
		duration_seconds: null,
		pad_minutes: null,
		folder_ids: []
	};
}

export function normalizePreset(preset: FillerPreset): FillerPreset {
	if (preset.mode === 'count') return { ...preset, count: preset.count ?? 1, duration_seconds: null, pad_minutes: null };
	if (preset.mode === 'duration')
		return { ...preset, count: null, duration_seconds: preset.duration_seconds ?? 60, pad_minutes: null };
	return { ...preset, count: null, duration_seconds: null, pad_minutes: preset.pad_minutes ?? 30 };
}

// ---- IPTV channels (Faza 20 sources) -------------------------------------------------

export const fetchLiveChannels = (token: string, libraryId: string) =>
	request<AdminLiveChannel[]>(
		token,
		`${ENDPOINTS.liveSources}/${encodeURIComponent(libraryId)}/channels`
	);

export const saveLiveChannel = (token: string, id: string, settings: ChannelSettings) =>
	request<void>(token, `${ENDPOINTS.liveSources}/channels/${encodeURIComponent(id)}`, 'PUT', settings);

/** A text field as the server wants it: empty means "no override". */
export const blankToNull = (value: string | null | undefined): string | null => {
	const text = (value ?? '').trim();
	return text === '' ? null : text;
};

/** M3U and XMLTV addresses to give an IPTV player (key placeholder when none yet). */
export function iptvLinks(origin: string, key: string | null) {
	const suffix = key ? `?key=${key}` : '';
	return {
		playlist: `${origin}/iptv/channels.m3u${suffix}`,
		guide: `${origin}/iptv/epg.xml${suffix}`
	};
}
