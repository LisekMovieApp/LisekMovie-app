import { describe, expect, it, vi } from 'vitest';
import { debounce, filterItems, fold } from './itemSearch';
import type { MediaItemSummary } from './mediaItems';

const item = (
	title: string,
	year: number | null,
	season: number | null = null,
	episode: number | null = null
): MediaItemSummary => ({
	id: `${title}-${season}-${episode}`,
	item_type: season === null ? 'movie' : 'episode',
	title,
	year,
	season,
	episode,
	metadata_locked: false,
	metadata_fallback: false
});

const items = [
	item('Różowa Pantera', 1963),
	item('Łowcy androidów', 1982),
	item('Lisek Show', null, 1, 2),
	item('Lisek Show', null, 2, 10)
];

describe('library item search (Faza 25)', () => {
	it('ignores case and Polish letters', () => {
		expect(fold('Łódź ŻÓŁW')).toBe('lodz zolw');
		expect(filterItems(items, 'rozowa').map((i) => i.title)).toEqual(['Różowa Pantera']);
		expect(filterItems(items, 'LOWCY').map((i) => i.title)).toEqual(['Łowcy androidów']);
	});

	it('matches every word, the year and episode codes', () => {
		expect(filterItems(items, 'pantera 1963')).toHaveLength(1);
		expect(filterItems(items, 'pantera 1999')).toHaveLength(0);
		expect(filterItems(items, 'lisek s01e02').map((i) => i.episode)).toEqual([2]);
		expect(filterItems(items, 's2e10').map((i) => i.episode)).toEqual([10]);
		expect(filterItems(items, '   ')).toBe(items);
	});

	it('waits for typing to stop', () => {
		vi.useFakeTimers();
		const fn = vi.fn();
		const run = debounce(fn, 200);
		run('a');
		run('ab');
		vi.advanceTimersByTime(199);
		expect(fn).not.toHaveBeenCalled();
		vi.advanceTimersByTime(1);
		expect(fn).toHaveBeenCalledExactlyOnceWith('ab');
		vi.useRealTimers();
	});
});
