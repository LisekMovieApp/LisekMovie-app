/**
 * Alpha picker logic (Faza 12): the letters a library grid can jump to and
 * where a jump lands. The server's jump table (`fetchLibraryLetters`) gives
 * each letter's first position in the whole, paged list; these helpers turn
 * that into a page and a card on it, and tell which letter a card belongs
 * to so the picker can highlight where the viewer is.
 */
import type { LibraryLetter } from './api';

/** Every picker entry, in display order: `#` (digits, symbols, other
 * scripts) first, then A–Z, like Jellyfin's picker. */
export const ALPHA_LETTERS: readonly string[] = [
	'#',
	...Array.from({ length: 26 }, (_, i) => String.fromCharCode(65 + i))
];

export interface JumpTarget {
	/** 1-based page of the paged grid. */
	page: number;
	/** 0-based card on that page. */
	offset: number;
}

/** Where `letter`'s first title sits, or `null` when it has no titles. */
export function jumpTarget(
	letters: readonly LibraryLetter[],
	letter: string,
	pageSize: number
): JumpTarget | null {
	const entry = letters.find((l) => l.letter === letter);
	if (!entry || pageSize < 1 || entry.index < 0) return null;
	return { page: Math.floor(entry.index / pageSize) + 1, offset: entry.index % pageSize };
}

/**
 * The letter whose run contains the card at `index` (0-based in the whole
 * list): the entry with the largest start not past it. `letters` is in
 * list order, as the server returns it, so this works for both title sorts.
 */
export function letterAt(letters: readonly LibraryLetter[], index: number): string | null {
	let found: string | null = null;
	let best = -1;
	for (const entry of letters) {
		if (entry.index <= index && entry.index > best) {
			best = entry.index;
			found = entry.letter;
		}
	}
	return found;
}
