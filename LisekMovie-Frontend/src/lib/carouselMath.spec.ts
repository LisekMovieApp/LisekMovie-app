import { describe, it, expect } from 'vitest';
import { clampOffset, computeMaxOffset, pageOffset } from './carouselMath';

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
