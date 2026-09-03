import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { animateScrollTo } from './scrollAnimation';

// No real DOM needed: `animateScrollTo` only touches `scrollLeft`,
// `scrollTo`, and `style.scrollSnapType`, plus the two browser globals
// stubbed below — so a bare fake object stands in for the element fine
// under this project's Node test environment. `scrollTo` here mimics real
// DOM behavior for an *instant* scroll only (`behavior: 'instant'`), which
// is all this function ever requests — see the `scrollTo` test below for
// why that specific option matters (and why `'auto'` does NOT work here).
function fakeElement(startLeft: number) {
	const scrollToCalls: Array<{ left: number; behavior: string }> = [];
	const el = {
		scrollLeft: startLeft,
		style: { scrollSnapType: 'x proximity' },
		scrollTo(opts: { left: number; behavior: ScrollBehavior }) {
			scrollToCalls.push({ left: opts.left, behavior: opts.behavior });
			el.scrollLeft = opts.left;
		}
	};
	return { el: el as unknown as HTMLElement, scrollToCalls };
}

describe('animateScrollTo', () => {
	let frames: FrameRequestCallback[];
	let now: number;

	beforeEach(() => {
		frames = [];
		now = 0;
		vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => {
			frames.push(cb);
			return frames.length;
		});
		vi.stubGlobal('performance', { now: () => now });
	});

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	function runFrame(atTime: number) {
		now = atTime;
		const due = frames;
		frames = [];
		for (const cb of due) cb(now);
	}

	it('does nothing when already at the target', () => {
		const { el } = fakeElement(100);
		animateScrollTo(el, 100);
		expect(frames).toHaveLength(0);
		expect(el.scrollLeft).toBe(100);
	});

	it('suspends scroll-snap for the duration and restores it afterward', () => {
		const { el } = fakeElement(0);
		animateScrollTo(el, 400, 200);
		expect(el.style.scrollSnapType).toBe('none');

		runFrame(100); // mid-animation
		expect(el.style.scrollSnapType).toBe('none');

		runFrame(200); // exactly at duration — animation completes
		expect(el.style.scrollSnapType).toBe('x proximity');
	});

	it('animates scrollLeft progressively toward the target, ending exactly on it', () => {
		const { el } = fakeElement(0);
		animateScrollTo(el, 1000, 200);

		runFrame(50);
		const midway = el.scrollLeft;
		expect(midway).toBeGreaterThan(0);
		expect(midway).toBeLessThan(1000);

		runFrame(150);
		const later = el.scrollLeft;
		expect(later).toBeGreaterThan(midway);
		expect(later).toBeLessThan(1000);

		runFrame(200);
		expect(el.scrollLeft).toBe(1000);
	});

	it('moves every frame via scrollTo({ behavior: "instant" }), never a plain scrollLeft assignment or "auto"', () => {
		// Regression test, in two layers — both were real bugs caught here
		// only after being measured against the actual running app, not
		// assumed from spec-reading:
		//
		// 1. A plain `el.scrollLeft = x` assignment is itself smoothly
		//    animated by the browser when the element's CSS
		//    `scroll-behavior` is `smooth` (as this project's carousels
		//    always have, for wheel/touch/keyboard scrolling) — so every
		//    per-frame update must go through `scrollTo` with an explicit
		//    `behavior`, or the animation barely moves for its whole
		//    intended duration and only catches up afterward.
		//
		// 2. `behavior: 'auto'` does NOT fix that: per the CSSOM View spec,
		//    `'auto'` means "use the element's computed `scroll-behavior`",
		//    so with `scroll-smooth` present it's smooth-animated exactly
		//    like no behavior at all — same bug, different-looking code.
		//    Only `behavior: 'instant'` unconditionally forces an instant
		//    jump regardless of the element's CSS.
		const { el, scrollToCalls } = fakeElement(0);
		animateScrollTo(el, 300, 200);
		runFrame(100);
		runFrame(200);

		expect(scrollToCalls.length).toBeGreaterThan(0);
		for (const call of scrollToCalls) {
			expect(call.behavior).toBe('instant');
		}
	});
});
