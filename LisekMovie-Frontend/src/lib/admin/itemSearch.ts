// Faza 25: the search box of a library's "Przeglądaj" list. Matching runs
// in the browser over the list already loaded: case- and accent-blind
// ("rozowa" finds "Różowa"), every word must match the title, the year or
// an episode code ("s01e02", "s1e2").
import type { MediaItemSummary } from './mediaItems';

/** Lower case without diacritics (ł counts as l). */
export function fold(text: string): string {
	return text.toLowerCase().normalize('NFD').replace(/\p{M}/gu, '').replace(/ł/g, 'l');
}

function haystack(item: MediaItemSummary): string {
	const parts = [item.title];
	if (item.year !== null) parts.push(String(item.year));
	if (item.season !== null && item.episode !== null) {
		const s = String(item.season);
		const e = String(item.episode);
		parts.push(`s${s.padStart(2, '0')}e${e.padStart(2, '0')}`, `s${s}e${e}`);
	}
	return fold(parts.join(' '));
}

/** The items whose title, year or episode code contain every word of `query`. */
export function filterItems(items: MediaItemSummary[], query: string): MediaItemSummary[] {
	const words = fold(query).split(/\s+/).filter(Boolean);
	if (words.length === 0) return items;
	return items.filter((item) => {
		const text = haystack(item);
		return words.every((word) => text.includes(word));
	});
}

/** Calls `fn` once `delay` ms passed since the last call. */
export function debounce<A extends unknown[]>(fn: (...args: A) => void, delay = 200) {
	let timer: ReturnType<typeof setTimeout> | undefined;
	const run = (...args: A) => {
		clearTimeout(timer);
		timer = setTimeout(() => fn(...args), delay);
	};
	run.cancel = () => clearTimeout(timer);
	return run;
}
