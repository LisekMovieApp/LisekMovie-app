import { describe, expect, it } from 'vitest';
import {
	audioOptionLabel,
	formatRating,
	formatRuntime,
	formatTimestamp,
	parseWatchQuery,
	preferredSubtitle,
	subtitleOptionLabel,
	watchQuery,
	type SubtitleOption
} from './details';

const words = { unknown: 'Nieznany', default: 'Domyślne', forced: 'Wymuszone' };

function sub(
	id: string,
	language: string | null,
	extra: Partial<SubtitleOption> = {}
): SubtitleOption {
	return {
		id,
		language,
		title: null,
		codec: 'subrip',
		kind: 'text',
		forced: false,
		is_default: false,
		external: false,
		...extra
	};
}

describe('detail page extras', () => {
	it('formats runtime, rating and scene times', () => {
		expect(formatRuntime(108)).toBe('1h 48m');
		expect(formatRuntime(48)).toBe('48m');
		expect(formatRuntime(null)).toBeNull();
		expect(formatRating(6.53)).toBe('6.5');
		expect(formatRating(0)).toBeNull();
		expect(formatTimestamp(65)).toBe('1:05');
		expect(formatTimestamp(3930)).toBe('1:05:30');
	});

	it('labels tracks the way the reference page does', () => {
		expect(
			audioOptionLabel(
				{ index: 1, language: 'pol', title: null, codec: 'ac3', channels: 6, is_default: true },
				'pl',
				words
			)
		).toBe('Polski - Dolby Digital - 5.1 - Domyślne');
		expect(
			audioOptionLabel(
				{
					index: 2,
					language: null,
					title: 'Lektor',
					codec: 'eac3',
					channels: 2,
					is_default: false
				},
				'pl',
				words
			)
		).toBe('Lektor - Dolby Digital+ - 2.0');
		expect(subtitleOptionLabel(sub('s3', 'pol', { forced: true }), 'pl', words)).toBe(
			'Polski - SRT - Wymuszone'
		);
	});

	it('preselects the last subtitle language, preferring full text tracks', () => {
		const tracks = [
			sub('s3', 'pol', { forced: true }),
			sub('s4', 'pol', { kind: 'image', codec: 'hdmv_pgs_subtitle' }),
			sub('x0', 'pol', { external: true }),
			sub('s5', 'eng')
		];
		expect(preferredSubtitle(tracks, 'pol')).toBe('x0');
		expect(preferredSubtitle(tracks, 'eng')).toBe('s5');
		expect(preferredSubtitle(tracks, 'ger')).toBe('');
		expect(preferredSubtitle(tracks, '')).toBe('');
		expect(preferredSubtitle(tracks, null)).toBe('');
	});

	it('round-trips the watch query', () => {
		const query = watchQuery({ audio: 2, subtitle: '', start: 301.7 });
		expect(query).toBe('?audio=2&sub=off&t=301');
		expect(parseWatchQuery(new URLSearchParams(query))).toEqual({
			audio: 2,
			subtitle: '',
			start: 301
		});
		expect(watchQuery({})).toBe('');
		expect(parseWatchQuery(new URLSearchParams('audio=x&sub=../../etc&t=-5'))).toEqual({
			audio: null,
			subtitle: null,
			start: null
		});
		expect(parseWatchQuery(new URLSearchParams('sub=s12')).subtitle).toBe('s12');
	});
});
