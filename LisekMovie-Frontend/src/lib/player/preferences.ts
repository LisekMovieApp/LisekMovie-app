/**
 * The viewer's player choices that should outlive one video: volume,
 * stereo mix, compatible playback, subtitle language and size. Kept per
 * browser in `localStorage`; a private window or blocked storage just
 * means defaults.
 */
import type { DownmixPreference } from '$lib/playback/api';

export type SubtitleSize = 'small' | 'medium' | 'large';

export interface PlayerPreferences {
	volume: number;
	muted: boolean;
	downmix: DownmixPreference;
	compatible: boolean;
	/** Language of the last subtitle picked; `''` = subtitles off; `null` = server default. */
	subtitleLanguage: string | null;
	subtitleSize: SubtitleSize;
}

const KEY = 'lisek.player';

export const DEFAULT_PREFERENCES: PlayerPreferences = {
	volume: 1,
	muted: false,
	downmix: 'auto',
	compatible: false,
	subtitleLanguage: null,
	subtitleSize: 'medium'
};

const DOWNMIX: DownmixPreference[] = ['auto', 'standard', 'dialogue', 'night', 'pro_logic_ii'];
const SIZES: SubtitleSize[] = ['small', 'medium', 'large'];

/** Accepts only well-formed values, so a hand-edited entry cannot break the player. */
export function parsePreferences(raw: string | null): PlayerPreferences {
	let data: Partial<Record<keyof PlayerPreferences, unknown>> = {};
	try {
		const parsed: unknown = raw ? JSON.parse(raw) : {};
		if (parsed && typeof parsed === 'object') data = parsed as typeof data;
	} catch {
		data = {};
	}
	const volume =
		typeof data.volume === 'number' && data.volume >= 0 && data.volume <= 1 ? data.volume : 1;
	return {
		volume,
		muted: data.muted === true,
		downmix: DOWNMIX.includes(data.downmix as DownmixPreference)
			? (data.downmix as DownmixPreference)
			: 'auto',
		compatible: data.compatible === true,
		subtitleLanguage:
			typeof data.subtitleLanguage === 'string' && data.subtitleLanguage.length <= 16
				? data.subtitleLanguage
				: null,
		subtitleSize: SIZES.includes(data.subtitleSize as SubtitleSize)
			? (data.subtitleSize as SubtitleSize)
			: 'medium'
	};
}

export function loadPreferences(): PlayerPreferences {
	try {
		return parsePreferences(localStorage.getItem(KEY));
	} catch {
		return { ...DEFAULT_PREFERENCES };
	}
}

export function savePreferences(preferences: PlayerPreferences) {
	try {
		localStorage.setItem(KEY, JSON.stringify(preferences));
	} catch {
		// Storage full or blocked: the choice lasts for this video only.
	}
}
