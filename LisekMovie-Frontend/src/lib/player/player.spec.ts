import { describe, expect, it, vi } from 'vitest';
import { parseCue } from './cues';
import { bufferedEnd, clampTime, endsAt, formatClock, formatTime } from './format';
import { commandOf, seekStep } from './keys';
import { audioTrackLabel, languageName, subtitleTrackLabel } from './labels';
import { parsePreferences } from './preferences';
import { repeatStep, SeekAccumulator } from './seek';
import { nextUpDue } from './nextUp';
import { thumbnailAt } from './trickplay';

const key = (key: string, keyCode = 0) => ({
	key,
	keyCode,
	altKey: false,
	ctrlKey: false,
	metaKey: false
});

describe('player helpers', () => {
	it('formats times', () => {
		expect(formatTime(0)).toBe('0:00');
		expect(formatTime(65.9)).toBe('1:05');
		expect(formatTime(3723)).toBe('1:02:03');
		expect(formatTime(Number.NaN)).toBe('0:00');
	});

	it('finds the buffered range around the position and clamps seeks', () => {
		const ranges = {
			length: 2,
			start: (i: number) => [0, 100][i],
			end: (i: number) => [30, 140][i]
		} as TimeRanges;
		expect(bufferedEnd(ranges, 10)).toBe(30);
		expect(bufferedEnd(ranges, 110)).toBe(140);
		expect(bufferedEnd(ranges, 50)).toBe(50);
		expect(clampTime(-5, 100)).toBe(0);
		expect(clampTime(500, 100)).toBe(99.5);
		expect(clampTime(42, Number.NaN)).toBe(42);
	});

	it('maps keyboard shortcuts and TV remote keys', () => {
		expect(commandOf(key(' '))).toBe('toggle');
		expect(commandOf(key('Unidentified', 10252))).toBe('toggle');
		expect(commandOf(key('Unidentified', 417))).toBe('forward_long');
		expect(commandOf(key('Unidentified', 461))).toBe('back');
		expect(commandOf({ ...key('f'), ctrlKey: true })).toBeNull();
		expect(commandOf(key('ArrowLeft'))).toBeNull();
		expect([seekStep('rewind'), seekStep('forward_long'), seekStep('mute')]).toEqual([-10, 30, 0]);
	});
});

describe('fast seeking', () => {
	it('holding a key speeds up from 10 s to 30 s to 60 s', () => {
		expect([0, 4, 5, 11, 12].map((r) => repeatStep(10, r))).toEqual([10, 10, 30, 30, 60]);
		expect(repeatStep(-10, 12)).toBe(-60);
	});

	it('adds presses up and seeks once, after the last one', () => {
		vi.useFakeTimers();
		const seeks: number[] = [];
		const shown: (number | null)[] = [];
		const seeker = new SeekAccumulator(
			(time) => seeks.push(time),
			(target) => shown.push(target),
			450
		);
		seeker.add(100, 10, 3600);
		seeker.add(100, 10, 3600);
		vi.advanceTimersByTime(300);
		expect(seeker.add(100, 10, 3600)).toBe(130);
		expect(seeks, 'no seek while presses keep coming').toEqual([]);
		vi.advanceTimersByTime(450);
		expect(seeks).toEqual([130]);
		expect(shown.at(-1)).toBeNull();
		// Clamped into the playable range.
		seeker.add(3590, 30, 3600);
		seeker.flush();
		expect(seeks.at(-1)).toBe(3599.5);
		// Dragging does not seek until released.
		seeker.set(50, 3600, false);
		vi.advanceTimersByTime(5000);
		expect(seeks.at(-1)).toBe(3599.5);
		seeker.flush();
		expect(seeks.at(-1)).toBe(50);
		vi.useRealTimers();
	});
});

describe('subtitle cues', () => {
	it('keeps italic, bold and underline and drops other markup', () => {
		expect(parseCue('<v Anna><i>Hello</i> &amp; <c.yellow>bye</c>\n<b>Line</b> two')).toEqual([
			[
				{ text: 'Hello', italic: true, bold: false, underline: false },
				{ text: ' & bye', italic: false, bold: false, underline: false }
			],
			[
				{ text: 'Line', italic: false, bold: true, underline: false },
				{ text: ' two', italic: false, bold: false, underline: false }
			]
		]);
		// Script-like text stays text: it is never rendered as HTML.
		expect(parseCue('&lt;script&gt;')[0][0].text).toBe('<script>');
	});
});

describe('preferences and labels', () => {
	it('rejects malformed stored preferences', () => {
		expect(parsePreferences('{"volume":7,"downmix":"loud","subtitleSize":"huge"}')).toMatchObject({
			volume: 1,
			downmix: 'auto',
			subtitleSize: 'medium'
		});
		expect(parsePreferences('not json').muted).toBe(false);
		expect(parsePreferences('{"subtitleLanguage":"pol","downmix":"night"}')).toMatchObject({
			subtitleLanguage: 'pol',
			downmix: 'night'
		});
	});

	it('names tracks in the viewer language', () => {
		expect(languageName('pol', 'pl')).toBe('Polski');
		expect(languageName('ger', 'en')).toBe('German');
		expect(languageName('und', 'en')).toBeNull();
		const audio = {
			index: 1,
			is_default: true,
			language: 'pol',
			title: 'Lektor',
			codec: 'eac3',
			channels: 6,
			channel_layout: null
		};
		expect(audioTrackLabel(audio, 'pl', '?')).toEqual({
			name: 'Polski – Lektor',
			detail: 'E-AC3 5.1'
		});
		const sub = {
			id: 's3',
			kind: 'image',
			forced: true,
			is_default: false,
			external: false,
			language: null,
			title: null,
			codec: 'hdmv_pgs_subtitle'
		};
		expect(subtitleTrackLabel(sub, 'en', 'Unknown', 'Forced')).toEqual({
			name: 'Unknown – Forced',
			detail: 'PGS'
		});
	});
});

describe('trickplay and next episode', () => {
	const manifest = {
		interval_seconds: 10,
		width: 320,
		height: 180,
		columns: 10,
		rows: 10,
		count: 250,
		sheets: [
			'/api/media/x/trickplay/0?v=1',
			'/api/media/x/trickplay/1?v=1',
			'/api/media/x/trickplay/2?v=1'
		]
	};

	it('finds the tile for a moment', () => {
		expect(thumbnailAt(manifest, 0, 't')).toMatchObject({
			x: 0,
			y: 0,
			url: '/api/media/x/trickplay/0?v=1&token=t'
		});
		// 1234 s: thumbnail 123 -> sheet 1, cell 23 -> column 3, row 2.
		expect(thumbnailAt(manifest, 1234, 't')).toMatchObject({
			x: 960,
			y: 360,
			sheetWidth: 3200,
			url: '/api/media/x/trickplay/1?v=1&token=t'
		});
		// Past the end: the last thumbnail, not a blank tile.
		expect(thumbnailAt(manifest, 99999, 't')?.url).toContain('/trickplay/2?');
		expect(thumbnailAt(manifest, Number.NaN, 't')).toBeNull();
	});

	it('shows the next-episode card for the credits or at the end', () => {
		expect(nextUpDue(30, 2400, false)).toBe(true);
		expect(nextUpDue(120, 2400, false)).toBe(false);
		expect(nextUpDue(30, 200, false), 'not before the end of a short extra').toBe(false);
		expect(nextUpDue(0, 200, true)).toBe(true);
	});
});

describe('end time', () => {
	it('is now plus what is left, and moves on while paused', () => {
		const start = Date.UTC(2026, 9, 26, 11, 0, 0);
		expect(endsAt(start, 75 * 60)?.getTime()).toBe(start + 75 * 60_000);
		// Paused for ten minutes: same remaining time, later end.
		expect(endsAt(start + 10 * 60_000, 75 * 60)?.getTime()).toBe(start + 85 * 60_000);
		expect(endsAt(start, Number.NaN)).toBeNull();
		expect(endsAt(start, -1)).toBeNull();
	});

	it('reads as a wall clock in the UI language', () => {
		const at = new Date(2026, 9, 26, 13, 5);
		expect(formatClock(at, 'pl')).toBe('13:05');
		expect(formatClock(at, 'en')).toMatch(/^1:05\s?PM$/);
	});
});
