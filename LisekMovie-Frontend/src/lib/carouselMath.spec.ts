import { describe, it, expect } from 'vitest';
import { clampOffset, computeMaxOffset, pageOffset, revealOffset } from './carouselMath';

describe('clampOffset', () => {
	it('keeps an in-range value unchanged', () => {
		expect(clampOffset(150, 400)).toBe(150);
	});

	it('clamps below zero up to zero', () => {
		expect(clampOffset(-50, 400)).toBe(0);
	});

	it('clamps above maxOffset down to maxOffset', () => {
		expect(clampOffset(999, 400)).toBe(400);
	});

	it('collapses to zero when there is no room to scroll at all', () => {
		expect(clampOffset(50, 0)).toBe(0);
		expect(clampOffset(50, -10)).toBe(0);
	});
});

describe('computeMaxOffset', () => {
	it('is the overhang of the track past the viewport', () => {
		expect(computeMaxOffset(1200, 800)).toBe(400);
	});

	it('is zero when the track fits entirely within the viewport', () => {
		expect(computeMaxOffset(600, 800)).toBe(0);
	});
});

describe('pageOffset', () => {
	it('advances forward by the page fraction of the viewport width', () => {
		expect(pageOffset(0, 1, 1000, 5000, 0.9)).toBe(900);
	});

	it('moves backward by the page fraction of the viewport width', () => {
		expect(pageOffset(1000, -1, 1000, 5000, 0.9)).toBe(100);
	});

	it('clamps a forward page past the end to maxOffset', () => {
		expect(pageOffset(4800, 1, 1000, 5000, 0.9)).toBe(5000);
	});

	it('clamps a backward page past the start to zero', () => {
		expect(pageOffset(50, -1, 1000, 5000, 0.9)).toBe(0);
	});
});

describe('revealOffset', () => {
	it('leaves a card that is already visible alone', () => {
		expect(revealOffset(100, 300, 500, 1000, 2000)).toBe(100);
	});

	it('slides right just enough to show a card past the right edge, plus a peek of the next', () => {
		expect(revealOffset(0, 900, 1100, 1000, 2000, 48)).toBe(148);
	});

	it('slides left to a card before the left edge, clamped at the start', () => {
		expect(revealOffset(600, 400, 600, 1000, 2000, 48)).toBe(352);
		expect(revealOffset(600, 10, 210, 1000, 2000, 48)).toBe(0);
	});

	it('never passes the end of the track', () => {
		expect(revealOffset(0, 2900, 3100, 1000, 2000)).toBe(2000);
	});
});
