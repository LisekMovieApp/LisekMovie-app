import { describe, expect, it } from 'vitest';
import { eta, percentDone, workerState, type TrickplayStatus } from './trickplay';

const base: TrickplayStatus = {
	enabled: true,
	paused_for_playback: false,
	ffmpeg_missing: false,
	ready: 10,
	failed: 0,
	pending: 5,
	libraries: [],
	failures: []
};

describe('trickplay admin helpers', () => {
	it('counts failures as done and rounds down', () => {
		expect(percentDone(1, 1, 3)).toBe(66);
		expect(percentDone(0, 0, 0)).toBe(100);
		expect(percentDone(299, 0, 300)).toBe(99);
	});

	it('formats a rough remaining time', () => {
		expect(eta(20)).toEqual({ kind: 'soon' });
		expect(eta(600)).toEqual({ kind: 'minutes', minutes: 10 });
		expect(eta(2 * 3600 + 15 * 60)).toEqual({ kind: 'hours', hours: 2, minutes: 15 });
	});

	it('tells what the worker is doing', () => {
		expect(workerState({ ...base, enabled: false })).toBe('disabled');
		expect(workerState({ ...base, ffmpeg_missing: true })).toBe('no_ffmpeg');
		expect(workerState({ ...base, paused_for_playback: true })).toBe('paused');
		expect(
			workerState({
				...base,
				current: { item_id: 'x', title: 'T', started_at: new Date().toISOString() }
			})
		).toBe('working');
		expect(workerState(base)).toBe('idle');
		expect(workerState({ ...base, pending: 0 })).toBe('done');
	});
});
