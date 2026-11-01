import { describe, expect, it } from 'vitest';
import { guideStart, programBox, progressOf, PX_PER_MINUTE } from './api';

describe('live TV guide layout', () => {
	const start = new Date('2026-11-01T18:00:00Z');
	const end = new Date('2026-11-01T20:00:00Z');

	it('places programmes on the time axis', () => {
		const box = programBox(
			{ starts_at: '2026-11-01T18:30:00Z', ends_at: '2026-11-01T19:00:00Z' },
			start,
			end
		);
		expect(box).toEqual({ left: 30 * PX_PER_MINUTE, width: 30 * PX_PER_MINUTE });
	});

	it('clips programmes running over the window edges and drops outside ones', () => {
		expect(
			programBox({ starts_at: '2026-11-01T17:00:00Z', ends_at: '2026-11-01T18:15:00Z' }, start, end)
		).toEqual({ left: 0, width: 15 * PX_PER_MINUTE });
		expect(
			programBox({ starts_at: '2026-11-01T21:00:00Z', ends_at: '2026-11-01T22:00:00Z' }, start, end)
		).toBeNull();
	});

	it('starts the window on a half hour before now', () => {
		const at = guideStart(new Date('2026-11-01T18:47:12'), 30);
		expect([at.getHours(), at.getMinutes(), at.getSeconds()]).toEqual([18, 0, 0]);
		const later = guideStart(new Date('2026-11-01T19:05:00'), 0);
		expect([later.getHours(), later.getMinutes()]).toEqual([19, 0]);
	});

	it('shows how much of the current programme has run', () => {
		const program = { starts_at: '2026-11-01T18:00:00Z', ends_at: '2026-11-01T19:00:00Z' };
		expect(progressOf(program, Date.parse('2026-11-01T18:15:00Z'))).toBe(0.25);
		expect(progressOf(program, Date.parse('2026-11-01T20:00:00Z'))).toBe(1);
	});
});
