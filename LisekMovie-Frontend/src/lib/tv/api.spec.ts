import { describe, expect, it } from 'vitest';
import {
	ALL_DAYS,
	dayOn,
	daysPreset,
	formatLength,
	iptvLinks,
	moveItem,
	newSlot,
	normalizeMode,
	slotProblem,
	toggleDay
} from './api';

describe('TV helpers (Faza 26)', () => {
	it('handles the weekday mask', () => {
		expect(dayOn(ALL_DAYS, 6)).toBe(true);
		expect(toggleDay(ALL_DAYS, 0)).toBe(126);
		expect(dayOn(toggleDay(0, 2), 2)).toBe(true);
		expect(daysPreset(127)).toBe('all');
		expect(daysPreset(0b0011111)).toBe('weekdays');
		expect(daysPreset(0b1100000)).toBe('weekend');
		expect(daysPreset(5)).toBeNull();
	});

	it('validates slots and clears fields of other modes', () => {
		const slot = newSlot({ id: 'x', kind: 'series' });
		expect(slotProblem(slot)).toBeNull();
		expect(slotProblem({ ...slot, start_time: '25:00' })).toBe('tv_admin.problem_time');
		expect(slotProblem({ ...slot, days_of_week: 0 })).toBe('tv_admin.problem_days');
		expect(slotProblem({ ...slot, mode: 'multiple', count: 0 })).toBe('tv_admin.problem_count');
		expect(slotProblem({ ...slot, mode: 'duration', duration_minutes: 2000 })).toBe('tv_admin.problem_duration');
		const multiple = normalizeMode({ ...slot, mode: 'multiple', count: null, duration_minutes: 30 });
		expect(multiple.count).toBe(1);
		expect(multiple.duration_minutes).toBeNull();
		const one = normalizeMode({ ...slot, mode: 'one', count: 3, duration_minutes: 30 });
		expect([one.count, one.duration_minutes]).toEqual([null, null]);
	});

	it('moves items, formats lengths and builds IPTV links', () => {
		expect(moveItem([1, 2, 3], 0, 2)).toEqual([2, 3, 1]);
		expect(moveItem([1, 2, 3], 1, 9)).toEqual([1, 2, 3]);
		expect(formatLength(3_930_000)).toBe('1:05:30');
		expect(formatLength(330_000)).toBe('5:30');
		expect(iptvLinks('http://h', 'k')).toEqual({
			playlist: 'http://h/iptv/channels.m3u?key=k',
			guide: 'http://h/iptv/epg.xml?key=k'
		});
		expect(iptvLinks('http://h', null).playlist).toBe('http://h/iptv/channels.m3u');
	});
});
