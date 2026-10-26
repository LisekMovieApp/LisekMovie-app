import { describe, expect, it } from 'vitest';
import { ALPHA_LETTERS, jumpTarget, letterAt } from './alpha';

const letters = [
	{ letter: '#', index: 0 },
	{ letter: 'A', index: 3 },
	{ letter: 'M', index: 150 },
	{ letter: 'Z', index: 299 }
];

describe('alpha picker', () => {
	it('lists # and A to Z', () => {
		expect(ALPHA_LETTERS).toHaveLength(27);
		expect(ALPHA_LETTERS[0]).toBe('#');
		expect(ALPHA_LETTERS[1]).toBe('A');
		expect(ALPHA_LETTERS[26]).toBe('Z');
	});

	it('turns a letter into a page and a card', () => {
		expect(jumpTarget(letters, 'A', 100)).toEqual({ page: 1, offset: 3 });
		expect(jumpTarget(letters, 'M', 100)).toEqual({ page: 2, offset: 50 });
		expect(jumpTarget(letters, 'Z', 100)).toEqual({ page: 3, offset: 99 });
		expect(jumpTarget(letters, 'Q', 100)).toBeNull();
		expect(jumpTarget(letters, 'A', 0)).toBeNull();
	});

	it('names the letter a card belongs to', () => {
		expect(letterAt(letters, 0)).toBe('#');
		expect(letterAt(letters, 2)).toBe('#');
		expect(letterAt(letters, 3)).toBe('A');
		expect(letterAt(letters, 200)).toBe('M');
		expect(letterAt([], 5)).toBeNull();
		// Descending order: the list starts with Z.
		const desc = [
			{ letter: 'Z', index: 0 },
			{ letter: 'A', index: 10 }
		];
		expect(letterAt(desc, 4)).toBe('Z');
		expect(letterAt(desc, 12)).toBe('A');
	});
});
