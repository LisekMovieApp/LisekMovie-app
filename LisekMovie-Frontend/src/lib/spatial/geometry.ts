/**
 * Pure geometry for spatial (arrow-key / TV remote) navigation: which
 * element lies "next" in a direction. Kept free of the DOM so it is unit
 * tested in Node.
 */

export type Direction = 'up' | 'down' | 'left' | 'right';

export interface Rect {
	left: number;
	top: number;
	right: number;
	bottom: number;
}

export interface Candidate<T> {
	item: T;
	rect: Rect;
}

/** Edges may touch or overlap by this much and still count as "beyond". */
const EDGE_TOLERANCE = 2;
/**
 * ...or by this share of the smaller element along the movement axis: a
 * focused card scaled up (TV focus style) overlaps its neighbours a little
 * and must not hide them from the next key press.
 */
const OVERLAP_SHARE = 0.2;
/** How much worse sideways misalignment is than distance straight ahead. */
const ALIGNMENT_WEIGHT = 2;

function overlap(a1: number, a2: number, b1: number, b2: number): number {
	return Math.min(a2, b2) - Math.max(a1, b1);
}

/**
 * The best candidate in `direction` from `from`, or `null`.
 *
 * A candidate must start beyond `from`'s edge in that direction. Among
 * those, the score is the gap straight ahead plus the sideways gap
 * (weighted), so the nearest element in the same row/column wins over a
 * closer one diagonally; ties go to the one whose centre is better
 * aligned.
 */
export function pickNext<T>(
	from: Rect,
	candidates: Candidate<T>[],
	direction: Direction
): T | null {
	const horizontal = direction === 'left' || direction === 'right';
	let best: { item: T; score: number; skew: number } | null = null;
	const center = { x: (from.left + from.right) / 2, y: (from.top + from.bottom) / 2 };
	for (const { item, rect } of candidates) {
		const size = horizontal
			? Math.min(from.right - from.left, rect.right - rect.left)
			: Math.min(from.bottom - from.top, rect.bottom - rect.top);
		const tolerance = Math.max(EDGE_TOLERANCE, size * OVERLAP_SHARE);
		const cx = (rect.left + rect.right) / 2;
		const cy = (rect.top + rect.bottom) / 2;
		let ahead: number;
		switch (direction) {
			case 'right':
				if (rect.left < from.right - tolerance || cx <= center.x) continue;
				ahead = rect.left - from.right;
				break;
			case 'left':
				if (rect.right > from.left + tolerance || cx >= center.x) continue;
				ahead = from.left - rect.right;
				break;
			case 'down':
				if (rect.top < from.bottom - tolerance || cy <= center.y) continue;
				ahead = rect.top - from.bottom;
				break;
			case 'up':
				if (rect.bottom > from.top + tolerance || cy >= center.y) continue;
				ahead = from.top - rect.bottom;
				break;
		}
		const shared = horizontal
			? overlap(from.top, from.bottom, rect.top, rect.bottom)
			: overlap(from.left, from.right, rect.left, rect.right);
		const sideways = Math.max(0, -shared);
		const score = Math.max(0, ahead) + ALIGNMENT_WEIGHT * sideways;
		const skew = horizontal
			? Math.abs((rect.top + rect.bottom) / 2 - (from.top + from.bottom) / 2)
			: Math.abs((rect.left + rect.right) / 2 - (from.left + from.right) / 2);
		if (!best || score < best.score || (score === best.score && skew < best.skew)) {
			best = { item, score, skew };
		}
	}
	return best?.item ?? null;
}

/** Arrow keys and the D-pad codes TV browsers send. */
export function directionOf(event: Pick<KeyboardEvent, 'key' | 'keyCode'>): Direction | null {
	switch (event.key) {
		case 'ArrowUp':
		case 'Up':
			return 'up';
		case 'ArrowDown':
		case 'Down':
			return 'down';
		case 'ArrowLeft':
		case 'Left':
			return 'left';
		case 'ArrowRight':
		case 'Right':
			return 'right';
	}
	switch (event.keyCode) {
		case 38:
			return 'up';
		case 40:
			return 'down';
		case 37:
			return 'left';
		case 39:
			return 'right';
	}
	return null;
}

/**
 * The remote's Back button: Tizen (10009), webOS (461), Android TV /
 * Fire TV (`GoBack`, `BrowserBack`). Escape and Backspace are handled by
 * the caller, which knows whether a text field has focus.
 */
export function isBackKey(event: Pick<KeyboardEvent, 'key' | 'keyCode'>): boolean {
	return (
		event.key === 'GoBack' ||
		event.key === 'BrowserBack' ||
		event.keyCode === 10009 ||
		event.keyCode === 461
	);
}
