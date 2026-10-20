/**
 * "Next episode" timing: when the card appears, when it counts down, and
 * the binge guard — after a few episodes started by the countdown alone,
 * with nobody touching anything, the next one waits for a press (the
 * viewer may have fallen asleep).
 */

/** The card appears this many seconds before the end (end credits). */
export const NEXT_UP_LEAD_SECONDS = 40;
/** Countdown before the next episode starts by itself. */
export const COUNTDOWN_SECONDS = 10;
/** Episodes in a row the countdown may start without any input. */
export const AUTO_ADVANCE_LIMIT = 3;
/** Shorter videos (trailers, extras) get no card before their end. */
const MIN_DURATION_FOR_LEAD = 300;

const KEY = 'lisek.autoAdvances';

export function nextUpDue(remaining: number, total: number, ended: boolean): boolean {
	if (ended) return true;
	return total >= MIN_DURATION_FOR_LEAD && remaining > 0 && remaining <= NEXT_UP_LEAD_SECONDS;
}

function read(): number {
	try {
		const value = Number(sessionStorage.getItem(KEY));
		return Number.isFinite(value) && value > 0 ? value : 0;
	} catch {
		return 0;
	}
}

function write(value: number) {
	try {
		sessionStorage.setItem(KEY, String(value));
	} catch {
		// Storage blocked: the guard just never triggers.
	}
}

/** The countdown may start the next episode by itself. */
export function autoAdvanceAllowed(): boolean {
	return read() < AUTO_ADVANCE_LIMIT;
}

/** The countdown (not the viewer) started an episode. */
export function recordAutoAdvance() {
	write(read() + 1);
}

/** Any key press, click or tap: someone is watching. */
export function resetAutoAdvances() {
	if (read() !== 0) write(0);
}
