import { describe, expect, it } from 'vitest';
import { directionOf, isBackKey, pickNext, type Rect } from './geometry';

const box = (left: number, top: number, width = 100, height = 60): Rect => ({
	left,
	top,
	right: left + width,
	bottom: top + height
});

// Two rows of three cards, 20 px apart, the second row shifted right by 50.
const grid = [
	{ item: 'a1', rect: box(0, 0) },
	{ item: 'a2', rect: box(120, 0) },
	{ item: 'a3', rect: box(240, 0) },
	{ item: 'b1', rect: box(50, 100) },
	{ item: 'b2', rect: box(170, 100) },
	{ item: 'b3', rect: box(290, 100) }
];

describe('pickNext', () => {
	it('moves along a row and stops at its end', () => {
		expect(pickNext(box(0, 0), grid.slice(1), 'right')).toBe('a2');
		expect(pickNext(box(120, 0), grid, 'left')).toBe('a1');
		expect(pickNext(box(240, 0), grid.slice(0, 3), 'right')).toBeNull();
	});

	it('goes to the best aligned card in the next row, not a diagonal one', () => {
		expect(pickNext(box(120, 0), grid, 'down')).toBe('b2');
		expect(pickNext(box(240, 0), grid, 'down')).toBe('b3');
		expect(pickNext(box(170, 100), grid, 'up')).toBe('a2');
	});

	it('reaches an element off to the side when nothing is straight ahead', () => {
		const navbar = [{ item: 'logo', rect: box(0, -200, 80, 40) }];
		expect(pickNext(box(600, 0), navbar, 'up')).toBe('logo');
		expect(pickNext(box(600, 0), navbar, 'down')).toBeNull();
	});

	it('maps arrows, legacy names, key codes and remote Back keys', () => {
		expect(directionOf({ key: 'ArrowLeft', keyCode: 37 })).toBe('left');
		expect(directionOf({ key: 'Down', keyCode: 0 })).toBe('down');
		expect(directionOf({ key: 'Unidentified', keyCode: 38 })).toBe('up');
		expect(directionOf({ key: 'Enter', keyCode: 13 })).toBeNull();
		expect(isBackKey({ key: 'Unidentified', keyCode: 10009 })).toBe(true);
		expect(isBackKey({ key: 'GoBack', keyCode: 0 })).toBe(true);
		expect(isBackKey({ key: 'Escape', keyCode: 27 })).toBe(false);
	});
});

describe('pickNext with a scaled-up focused card', () => {
	it('still reaches the neighbour its focus scale overlaps', () => {
		const rect = (left: number, width = 288): Rect => ({
			left,
			top: 0,
			right: left + width,
			bottom: 162
		});
		// Card at 0..288 scaled by 7% overlaps the next card (296..584) by ~2 px.
		const focused = { left: -10, top: -6, right: 298, bottom: 168 };
		const next = pickNext(
			focused,
			[
				{ item: 'second', rect: rect(296) },
				{ item: 'third', rect: rect(592) }
			],
			'right'
		);
		expect(next).toBe('second');
	});
});
