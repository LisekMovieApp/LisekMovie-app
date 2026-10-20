/**
 * Track names the way VOD apps show them: the language in the viewer's
 * own language ("Polski", "English"), the file's track title next to it
 * ("Lektor"), technical details (codec, channels) in a quieter second line.
 */
import type { AudioTrack, SubtitleTrack } from '$lib/playback/api';

const LAYOUTS: Record<number, string> = { 1: '1.0', 2: '2.0', 6: '5.1', 7: '6.1', 8: '7.1' };
const CODECS: Record<string, string> = {
	hdmv_pgs_subtitle: 'PGS',
	dvd_subtitle: 'VobSub',
	subrip: 'SRT',
	webvtt: 'VTT',
	mov_text: 'TX3G',
	eac3: 'E-AC3',
	truehd: 'TrueHD'
};

const names = new Map<string, Intl.DisplayNames | null>();

/** "pol" / "pl" -> "polski" in Polish, "Polish" in English; null when unknown. */
export function languageName(code: string | null | undefined, locale: string): string | null {
	if (!code || code === 'und' || code === 'zxx') return null;
	if (!names.has(locale)) {
		try {
			names.set(locale, new Intl.DisplayNames([locale], { type: 'language', fallback: 'none' }));
		} catch {
			names.set(locale, null);
		}
	}
	try {
		const name = names.get(locale)?.of(code);
		if (name) return name.charAt(0).toLocaleUpperCase(locale) + name.slice(1);
	} catch {
		// Not a language code Intl knows (custom tags in old files).
	}
	return code.toUpperCase();
}

export interface TrackLabel {
	name: string;
	detail: string;
}

function codec(value: string | null | undefined): string {
	return value ? (CODECS[value] ?? value.toUpperCase()) : '';
}

/** `unknown` is the translated word for a track without a language. */
export function audioTrackLabel(track: AudioTrack, locale: string, unknown: string): TrackLabel {
	const language = languageName(track.language, locale);
	const name = [language ?? (track.title ? null : unknown), track.title]
		.filter(Boolean)
		.join(' – ');
	const channels = track.channels ? (LAYOUTS[track.channels] ?? `${track.channels}ch`) : '';
	return { name, detail: [codec(track.codec), channels].filter(Boolean).join(' ') };
}

/** `forced` is the translated word ("Forced"/"Wymuszone"). */
export function subtitleTrackLabel(
	track: SubtitleTrack,
	locale: string,
	unknown: string,
	forced: string
): TrackLabel {
	const language = languageName(track.language, locale);
	const parts = [
		language ?? (track.title ? null : unknown),
		track.title,
		track.forced ? forced : null
	];
	return { name: parts.filter(Boolean).join(' – '), detail: codec(track.codec) };
}
