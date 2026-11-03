/**
 * The viewer's player choices that should outlive one video: volume,
 * stereo mix, subtitle language, size and background, picture quality.
 * Faza 23: compatible playback is no longer remembered (the quality
 * presets replace it; it stays a one-off retry after an error). Kept per
 * browser in `localStorage`; a private window or blocked storage just
 * means defaults.
 */
import type { DownmixPreference } from '$lib/playback/api';
import { isQualityId, type QualityId } from './quality';

export type SubtitleSize = 'small' | 'medium' | 'large';
/** Faza 23: opacity of the black box behind subtitles, in percent. */
export type SubtitleBackground = 0 | 25 | 50 | 75 | 100;
export const SUBTITLE_BACKGROUNDS: readonly SubtitleBackground[] = [0, 25, 50, 75, 100];

export interface PlayerPreferences {
	volume: number;
	muted: boolean;
	downmix: DownmixPreference;
	/** Language of the last subtitle picked; `''` = subtitles off; `null` = server default. */
	subtitleLanguage: string | null;
	subtitleSize: SubtitleSize;
	subtitleBackground: SubtitleBackground;
	/** Faza 20: picture quality picked in the player. */
	quality: QualityId;
}

const KEY = 'lisek.player';

export const DEFAULT_PREFERENCES: PlayerPreferences = {
	volume: 1,
	muted: false,
	downmix: 'auto',
	subtitleLanguage: null,
	subtitleSize: 'medium',
	subtitleBackground: 25,
	quality: 'original'
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
		subtitleLanguage:
			typeof data.subtitleLanguage === 'string' && data.subtitleLanguage.length <= 16
				? data.subtitleLanguage
				: null,
		subtitleSize: SIZES.includes(data.subtitleSize as SubtitleSize)
			? (data.subtitleSize as SubtitleSize)
			: 'medium',
		subtitleBackground: SUBTITLE_BACKGROUNDS.includes(data.subtitleBackground as SubtitleBackground)
			? (data.subtitleBackground as SubtitleBackground)
			: 25,
		quality: isQualityId(data.quality) ? data.quality : 'original'
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
