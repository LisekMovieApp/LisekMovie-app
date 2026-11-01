import { describe, expect, it } from 'vitest';
import { isQualityId, qualityLimits, QUALITIES } from './quality';
import { parsePreferences } from './preferences';

describe('picture quality (Faza 20)', () => {
	it('offers the original and four presets, largest first', () => {
		expect(QUALITIES.map((q) => q.id)).toEqual(['original', '1080p', '720p', '480p', '360p']);
		expect(QUALITIES.map((q) => q.maxBitrateKbps)).toEqual([null, 8000, 4000, 2000, 1000]);
	});

	it('turns a preset into the start request limits', () => {
		expect(qualityLimits('original')).toEqual({ max_height: null, max_bitrate_kbps: null });
		expect(qualityLimits('720p')).toEqual({ max_height: 720, max_bitrate_kbps: 4000 });
		expect(qualityLimits('360p')).toEqual({ max_height: 360, max_bitrate_kbps: 1000 });
	});

	it('is remembered per browser and survives a bad value', () => {
		expect(parsePreferences(JSON.stringify({ quality: '480p' })).quality).toBe('480p');
		expect(parsePreferences(JSON.stringify({ quality: '4k' })).quality).toBe('original');
		expect(parsePreferences(null).quality).toBe('original');
		expect(isQualityId('1080p')).toBe(true);
		expect(isQualityId(1080)).toBe(false);
	});
});
