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

describe('own channels among the IPTV ones', () => {
	it('shows an own channel like a live one, with what is on now and next', async () => {
		const { fromOwn } = await import('./api');
		const channel = fromOwn({
			id: 'c1',
			number: '5',
			name: 'Lisek Kino',
			has_logo: true,
			now: { title: 'Film', starts_at: '2026-11-01T18:00:00Z', ends_at: '2026-11-01T19:30:00Z' },
			next: { title: 'Serial', subtitle: 'S01E01', starts_at: '2026-11-01T19:30:00Z', ends_at: '2026-11-01T20:00:00Z' }
		});
		expect(channel.own).toBe(true);
		expect(channel.logo_url).toBe('/api/tv/channels/c1/logo');
		expect(channel.now?.title).toBe('Film');
		expect(channel.next?.subtitle).toBe('S01E01');
		expect(fromOwn({ id: 'c2', number: '6', name: 'X', has_logo: false }).logo_url).toBeNull();
	});

	it('lists own channels first and survives one source failing', async () => {
		const { mergeSettled } = await import('./api');
		expect(await mergeSettled(Promise.resolve([1]), Promise.resolve([2, 3]))).toEqual([1, 2, 3]);
		expect(await mergeSettled(Promise.reject(new Error('x')), Promise.resolve([2]))).toEqual([2]);
		expect(await mergeSettled(Promise.resolve([1]), Promise.reject(new Error('x')))).toEqual([1]);
		await expect(mergeSettled(Promise.reject(new Error('a')), Promise.reject(new Error('b')))).rejects.toThrow('b');
	});
});
