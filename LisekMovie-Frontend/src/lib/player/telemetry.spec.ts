import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ProgressReporter, type MediaSnapshot, type PlaybackProgress } from './telemetry';

let media: MediaSnapshot;
let clock: number;
let sent: { report: PlaybackProgress; final: boolean }[];

function reporter(next = 15) {
	return new ProgressReporter({
		sessionId: 's1',
		durationHint: 6000,
		snapshot: () => media,
		now: () => clock,
		send: async (report, final) => {
			sent.push({ report, final });
			return {
				resume_position_seconds: report.position_seconds,
				watched: false,
				next_report_seconds: next
			};
		}
	});
}

beforeEach(() => {
	vi.useFakeTimers();
	media = {
		currentTime: 2700,
		duration: Number.NaN,
		paused: false,
		bandwidthKbps: 24000.4,
		droppedFrames: 3
	};
	clock = 0;
	sent = [];
});
afterEach(() => vi.useRealTimers());

describe('ProgressReporter', () => {
	it('sends heartbeats on the interval the server asks for', async () => {
		const r = reporter(30);
		r.start();
		await vi.advanceTimersByTimeAsync(15_000);
		expect(sent).toHaveLength(1);
		expect(sent[0].report).toMatchObject({
			event: 'heartbeat',
			position_seconds: 2700,
			duration_seconds: 6000,
			bandwidth_kbps: 24000
		});
		await vi.advanceTimersByTimeAsync(15_000);
		expect(sent, 'next one after 30 s').toHaveLength(1);
		await vi.advanceTimersByTimeAsync(15_000);
		expect(sent).toHaveLength(2);
	});

	it('counts stalls but not seeks, and reports deltas once', async () => {
		const r = reporter();
		r.onWaiting();
		clock = 1500;
		r.onPlaying();
		r.onSeeking();
		r.onWaiting();
		clock = 9000;
		r.onPlaying();
		r.onSeeked();
		await vi.runOnlyPendingTimersAsync();
		expect(sent[0].report).toMatchObject({ event: 'seek', stalls: 1, stall_seconds: 1.5 });
		expect(sent[1].report).toMatchObject({ stalls: 0, stall_seconds: 0 });
	});

	it('ends with one final report and ignores later events', async () => {
		const r = reporter();
		r.start();
		media.paused = true;
		r.onPause();
		await r.stop();
		r.onPlay();
		await vi.advanceTimersByTimeAsync(60_000);
		expect(sent.map((s) => [s.report.event, s.final])).toEqual([
			['pause', false],
			['stop', true]
		]);
		expect(r.stopped).toBe(true);
	});

	it('reports the end of the film instead of the pause before it', async () => {
		const r = reporter();
		media = { currentTime: 5999.8, duration: 6000, paused: true };
		r.onPause();
		r.onEnded();
		await vi.runOnlyPendingTimersAsync();
		expect(sent.map((s) => s.report.event)).toEqual(['ended']);
	});
});
