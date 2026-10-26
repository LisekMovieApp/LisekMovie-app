import { describe, expect, it } from 'vitest';
import { episodeEndsAt, episodeHeading, progressOf, seasonTitle } from './series';

const t = (key: string, params?: Record<string, string | number>) =>
	key === 'series.specials'
		? 'Odcinki specjalne'
		: key === 'detail.season_label'
			? `Sezon ${params?.number}`
			: `Odcinek ${params?.number}`;

describe('series pages', () => {
	it('names seasons and episodes', () => {
		expect(seasonTitle({ season: 1, name: 'Sezon 1: Początek' }, t)).toBe('Sezon 1: Początek');
		expect(seasonTitle({ season: 2, name: null }, t)).toBe('Sezon 2');
		expect(seasonTitle({ season: 0 }, t)).toBe('Odcinki specjalne');
		expect(episodeHeading({ episode: 1, end_episode: null, title: 'Najwyższy czas' }, t)).toBe(
			'1. Najwyższy czas'
		);
		expect(episodeHeading({ episode: 1, end_episode: 2, title: 'A / B' }, t)).toBe('1-2. A / B');
		expect(episodeHeading({ episode: 3, end_episode: null, title: null }, t)).toBe('3. Odcinek 3');
	});

	it('shows progress only for a started, unfinished episode', () => {
		expect(progressOf({ position_seconds: 300, duration_seconds: 3000, watched: false })).toBe(0.1);
		expect(progressOf({ position_seconds: 3, duration_seconds: 3000, watched: false })).toBeNull();
		expect(progressOf({ position_seconds: 300, duration_seconds: 3000, watched: true })).toBeNull();
		expect(
			progressOf({ position_seconds: 300, duration_seconds: null, watched: false })
		).toBeNull();
	});

	it('ends an episode from now, or from its resume point', () => {
		const now = Date.UTC(2026, 9, 26, 12, 0);
		expect(
			episodeEndsAt(now, { runtime_minutes: 51, position_seconds: 0, watched: false })?.getTime()
		).toBe(now + 51 * 60_000);
		expect(
			episodeEndsAt(now, { runtime_minutes: 51, position_seconds: 600, watched: false })?.getTime()
		).toBe(now + 41 * 60_000);
		expect(
			episodeEndsAt(now, { runtime_minutes: null, position_seconds: 0, watched: false })
		).toBeNull();
	});
});
