// Manual, browser-independent smooth-scroll for the media-row carousels'
// arrow buttons (`MediaRow.svelte`, `Top10Row.svelte`).
//
// Deliberately NOT `element.scrollBy({ behavior: 'smooth' })`: that depends
// on the browser's own native smooth-scroll implementation, which — at
// least in Chromium — doesn't reliably animate on an element that also has
// `scroll-snap-type` set. The snap correction can cut the animation short
// into an instant jump instead of letting it play. That was the first fix
// tried here (plus switching `snap-mandatory` to the less aggressive
// `snap-proximity`), and it measured as smooth in this project's own
// headless-Chromium testing — but was still reported as an instant jump on
// a real desktop Chrome, meaning the native-smooth-scroll approach isn't
// reliable enough across environments to trust as-is.
//
// This drives the scroll position itself, frame by frame, rather than
// asking the browser to animate toward a target — there is no native
// animation for scroll-snap to interrupt. One easy-to-miss trap doing that:
// the container also has `scroll-behavior: smooth` (the `scroll-smooth`
// Tailwind class, kept so wheel/touch/keyboard scrolling still glides) —
// and per the CSSOM View spec, once that's set, even a plain `el.scrollLeft
// = x` PROPERTY ASSIGNMENT is animated smoothly by the browser rather than
// applied instantly. Left alone, that meant every one of this function's
// ~23 per-frame assignments kicked off its own overlapping native smooth
// scroll, each retargeted before the last one finished — the visible
// result was barely-there motion for the whole intended duration, then a
// final catch-up once this loop stopped reassigning it.
//
// The first attempt at a fix swapped the assignment for `el.scrollTo({
// left, behavior: 'auto' })`, on the assumption that an explicit `behavior`
// always overrides the element's CSS `scroll-behavior`. Measured directly
// (an in-page diagnostic recording `scrollLeft` on every rAF frame), that
// did NOT fix it — the exact same flat-then-jump curve remained. Per the
// CSSOM View spec, `behavior: 'auto'` doesn't force instant scrolling at
// all: it explicitly means "use the computed `scroll-behavior` of the
// element", so with `scroll-smooth` in play it's smooth-animated same as
// no behavior at all. The value that actually forces an instant, unanimated
// jump regardless of the element's CSS is `behavior: 'instant'` — that's
// what this uses, and it was re-verified with the same frame-by-frame
// diagnostic to produce a continuously progressing curve from frame 1
// rather than a long flat stretch followed by a compressed catch-up.
//
// As a side effect this also ignores `prefers-reduced-motion` (which the
// native smooth-scroll API would have respected) — acceptable here, since
// this is a short, deliberate, user-triggered UI transition, not
// decorative motion.
export function animateScrollTo(el: HTMLElement, targetLeft: number, duration = 380): void {
	const startLeft = el.scrollLeft;
	const distance = targetLeft - startLeft;
	if (distance === 0) return;

	// Scroll-snap fights a programmatic scroll position the same way it
	// fights a native smooth `scrollBy` — suspend it for the length of this
	// animation and restore it once done, so wheel/touch/keyboard scrolling
	// still snaps normally afterward.
	const previousSnapType = el.style.scrollSnapType;
	el.style.scrollSnapType = 'none';

	const startTime = performance.now();

	function step(now: number) {
		const elapsed = Math.min((now - startTime) / duration, 1);
		const eased = 1 - Math.pow(1 - elapsed, 3); // ease-out cubic
		el.scrollTo({ left: startLeft + distance * eased, behavior: 'instant' });

		if (elapsed < 1) {
			requestAnimationFrame(step);
		} else {
			el.style.scrollSnapType = previousSnapType;
		}
	}

	requestAnimationFrame(step);
}
