import { describe, expect, it } from 'vitest';
import { dashboardQuery, humanDuration, lastDays, percent, progressOf } from './stats';

const filters = {
	from: '2026-09-14',
	to: '2026-09-20',
	userId: '',
	platform: '',
	deviceType: '',
	content: '',
	item: ''
};

describe('stats dashboard helpers', () => {
	it('formats durations like the bot reports', () => {
		expect(humanDuration(5880)).toBe('1h 38m');
		expect(humanDuration(528)).toBe('8m 48s');
		expect(humanDuration(45.4)).toBe('45s');
		expect(humanDuration(-1)).toBe('0s');
		expect(humanDuration(Number.NaN)).toBe('0s');
	});

	it('builds the query for whole local days', () => {
		const query = new URLSearchParams(dashboardQuery(filters, 'Europe/Warsaw') ?? '');
		const from = new Date(query.get('from') ?? '');
		const to = new Date(query.get('to') ?? '');
		expect((to.getTime() - from.getTime()) / 86_400_000).toBeCloseTo(7, 1);
		expect(from.getDate()).toBe(14);
		expect(query.get('tz')).toBe('Europe/Warsaw');
		expect(query.has('platform')).toBe(false);
		const narrowed = new URLSearchParams(
			dashboardQuery({ ...filters, platform: 'smart_tv', item: 'abc' }, 'UTC', { offset: '50' }) ??
				''
		);
		expect([narrowed.get('platform'), narrowed.get('item'), narrowed.get('offset')]).toEqual([
			'smart_tv',
			'abc',
			'50'
		]);
		expect(dashboardQuery({ ...filters, to: '2026-09-01' }, 'UTC')).toBeNull();
		expect(dashboardQuery({ ...filters, from: 'nope' }, 'UTC')).toBeNull();
	});

	it('computes ranges and shares', () => {
		expect(lastDays(7, new Date(2026, 9, 27))).toEqual({ from: '2026-10-21', to: '2026-10-27' });
		expect(progressOf({ position_seconds: 40, duration_seconds: 100 })).toBe(0.4);
		expect(progressOf({ position_seconds: 400, duration_seconds: 100 })).toBe(1);
		expect(progressOf({ position_seconds: 40, duration_seconds: 0 })).toBeNull();
		expect(percent(1, 3)).toBe(33);
		expect(percent(1, 0)).toBe(0);
	});
});
