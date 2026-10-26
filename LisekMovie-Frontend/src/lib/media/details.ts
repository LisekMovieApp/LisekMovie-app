// The detail page's extras (Faza 9): `GET /api/media/{id}/details` —
// original title, tagline, rating, genres, tags, crew, the streams to pick
// before playback and the chapters ("Sceny").

import type { components } from '$lib/api/schema';
import { buildAuthHeader } from '$lib/auth';
import { languageName } from '$lib/player/labels';

export type ItemDetails = components['schemas']['ItemDetailsResponse'];
export type AudioOption = components['schemas']['AudioOption'];
export type SubtitleOption = components['schemas']['SubtitleOption'];
export type Chapter = components['schemas']['ChapterResponse'];

export async function fetchItemDetails(token: string, itemId: string): Promise<ItemDetails> {
	const url = `/api/media/${encodeURIComponent(itemId)}/details`;
	const response = await fetch(url, { headers: buildAuthHeader(token) });
	if (!response.ok) throw new Error(`GET ${url} failed with status ${response.status}`);
	return (await response.json()) as ItemDetails;
}

/** "1h 48m", "48m"; `null` for an unknown length. */
export function formatRuntime(minutes: number | null | undefined): string | null {
	if (!minutes || minutes <= 0) return null;
	const h = Math.floor(minutes / 60);
	const m = minutes % 60;
	return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/** "1:05:30" / "12:05": where a scene starts. */
export function formatTimestamp(seconds: number): string {
	const total = Math.max(0, Math.floor(seconds));
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = String(total % 60).padStart(2, '0');
	return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

/** "6.5" — one decimal, as TMDB shows its score. */
export function formatRating(rating: number | null | undefined): string | null {
	return rating && rating > 0 ? rating.toFixed(1) : null;
}

const AUDIO_CODECS: Record<string, string> = {
	ac3: 'Dolby Digital',
	eac3: 'Dolby Digital+',
	truehd: 'Dolby TrueHD',
	dts: 'DTS',
	aac: 'AAC',
	mp3: 'MP3',
	mp2: 'MP2',
	opus: 'Opus',
	vorbis: 'Vorbis',
	flac: 'FLAC',
	alac: 'ALAC'
};
const LAYOUTS: Record<number, string> = { 1: 'Mono', 2: '2.0', 6: '5.1', 7: '6.1', 8: '7.1' };
const SUBTITLE_CODECS: Record<string, string> = {
	hdmv_pgs_subtitle: 'PGS',
	dvd_subtitle: 'VobSub',
	dvb_subtitle: 'DVB',
	subrip: 'SRT',
	srt: 'SRT',
	webvtt: 'VTT',
	vtt: 'VTT',
	mov_text: 'TX3G',
	ass: 'ASS',
	ssa: 'SSA',
	sup: 'PGS'
};

export interface LabelWords {
	unknown: string;
	default: string;
	forced: string;
}

/** "Polski - Lektor - Dolby Digital - 5.1 - Domyślne". */
export function audioOptionLabel(track: AudioOption, locale: string, words: LabelWords): string {
	const codec = track.codec ? (AUDIO_CODECS[track.codec] ?? track.codec.toUpperCase()) : null;
	const layout = track.channels ? (LAYOUTS[track.channels] ?? `${track.channels}ch`) : null;
	return [
		languageName(track.language, locale) ?? (track.title ? null : words.unknown),
		track.title,
		codec,
		layout,
		track.is_default ? words.default : null
	]
		.filter(Boolean)
		.join(' - ');
}

/** "Polski - SRT - Wymuszone". */
export function subtitleOptionLabel(
	track: SubtitleOption,
	locale: string,
	words: LabelWords
): string {
	const codec = track.codec ? (SUBTITLE_CODECS[track.codec] ?? track.codec.toUpperCase()) : null;
	return [
		languageName(track.language, locale) ?? (track.title ? null : words.unknown),
		track.title,
		codec,
		track.forced ? words.forced : null,
		track.is_default ? words.default : null
	]
		.filter(Boolean)
		.join(' - ');
}

/**
 * The subtitle preselected on the page: the language picked last time in
 * the player (`''` there = off), a full (not forced) track preferred and
 * text over pictures. `''` = off.
 */
export function preferredSubtitle(tracks: SubtitleOption[], language: string | null): string {
	if (!language) return '';
	const matching = tracks.filter((t) => t.language === language && !t.forced);
	return (matching.find((t) => t.kind === 'text') ?? matching[0])?.id ?? '';
}

/** Query string for `/watch/[id]`: the picked streams and a start time. */
export function watchQuery(options: {
	audio?: number | null;
	subtitle?: string | null;
	start?: number | null;
}): string {
	const params = new URLSearchParams();
	if (options.audio !== null && options.audio !== undefined)
		params.set('audio', String(options.audio));
	if (options.subtitle !== null && options.subtitle !== undefined)
		params.set('sub', options.subtitle === '' ? 'off' : options.subtitle);
	if (options.start !== null && options.start !== undefined)
		params.set('t', String(Math.max(0, Math.floor(options.start))));
	const query = params.toString();
	return query ? `?${query}` : '';
}

/** What `/watch/[id]` reads back: `null` = not given (server default). */
export function parseWatchQuery(params: URLSearchParams): {
	audio: number | null;
	subtitle: string | null;
	start: number | null;
} {
	const audioRaw = params.get('audio');
	const audio = audioRaw !== null && /^\d{1,4}$/.test(audioRaw) ? Number(audioRaw) : null;
	const sub = params.get('sub');
	const subtitle = sub === 'off' ? '' : sub !== null && /^[sx]\d{1,4}$/.test(sub) ? sub : null;
	const startRaw = params.get('t');
	const start = startRaw !== null && /^\d{1,6}(\.\d+)?$/.test(startRaw) ? Number(startRaw) : null;
	return { audio, subtitle, start };
}
