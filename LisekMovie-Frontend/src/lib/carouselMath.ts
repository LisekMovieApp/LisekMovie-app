// Pure offset arithmetic for `Carousel.svelte`, kept separate from the
// component so it's testable without a DOM (this project's vitest setup has
// no jsdom/happy-dom — see `scrollAnimation.spec.ts`'s old comment, now
// removed along with that module, for why that constraint shaped things
// this way previously too).

/** Clamps a track offset (in pixels) to the valid `[0, maxOffset]` range. */
export function clampOffset(offset: number, maxOffset: number): number {
	if (maxOffset <= 0) return 0;
	return Math.max(0, Math.min(offset, maxOffset));
}

/** How far the track can slide before its trailing edge reaches the
 * viewport's trailing edge — zero or negative means nothing to scroll. */
export function computeMaxOffset(trackWidth: number, viewportWidth: number): number {
	return Math.max(0, trackWidth - viewportWidth);
}

/** The clamped offset after an arrow-button page-scroll in `direction`. */
export function pageOffset(
	currentOffset: number,
	direction: 1 | -1,
	viewportWidth: number,
	maxOffset: number,
	pageFraction = 0.9
): number {
	return clampOffset(currentOffset + direction * viewportWidth * pageFraction, maxOffset);
}

/**
 * The offset that brings one card (`start`..`end`, in track pixels) fully
 * into the visible window, with `margin` pixels of its neighbours still
 * showing, so focus moved by keyboard or a TV remote never lands on a card
 * hidden past the edge. Unchanged when the card is already in view.
 */
export function revealOffset(
	currentOffset: number,
	start: number,
	end: number,
	visibleWidth: number,
	maxOffset: number,
	margin = 48
): number {
	if (start - margin < currentOffset) return clampOffset(start - margin, maxOffset);
	if (end + margin > currentOffset + visibleWidth) {
		return clampOffset(end + margin - visibleWidth, maxOffset);
	}
	return currentOffset;
}
