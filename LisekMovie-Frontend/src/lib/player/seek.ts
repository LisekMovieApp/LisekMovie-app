/**
 * Fast seeking: presses add up to one target and the player seeks once,
 * a moment after the last press. With on-demand HLS every real seek can
 * restart the server's encoder at the new position, so ten presses of
 * "+10 s" must become one seek of +100 s, not ten.
 */
import { clampTime } from './format';

/** How far one press moves while a key is held: 10 s, then 30 s, then 60 s. */
export function repeatStep(step: number, repeats: number): number {
	const size = Math.abs(step);
	const scaled = repeats >= 12 ? Math.max(size, 60) : repeats >= 5 ? Math.max(size, 30) : size;
	return Math.sign(step) * scaled;
}

export class SeekAccumulator {
	#target: number | null = null;
	#repeats = 0;
	#timer: ReturnType<typeof setTimeout> | undefined;
	#commit: (time: number) => void;
	#onChange: (target: number | null) => void;
	#delay: number;

	constructor(
		commit: (time: number) => void,
		onChange: (target: number | null) => void = () => {},
		delay = 450
	) {
		this.#commit = commit;
		this.#onChange = onChange;
		this.#delay = delay;
	}

	get target() {
		return this.#target;
	}

	/** Adds a step to the pending target (starting from `from`); returns the new target. */
	add(from: number, step: number, duration: number, repeat = false): number {
		this.#repeats = repeat ? this.#repeats + 1 : 0;
		const base = this.#target ?? from;
		this.#target = clampTime(base + repeatStep(step, this.#repeats), duration);
		this.#onChange(this.#target);
		this.#arm();
		return this.#target;
	}

	/** Sets an absolute target (the seek bar); `arm` = false while dragging, the release commits. */
	set(time: number, duration: number, arm = true) {
		this.#target = clampTime(time, duration);
		this.#onChange(this.#target);
		if (arm) this.#arm();
		else clearTimeout(this.#timer);
	}

	#arm() {
		clearTimeout(this.#timer);
		this.#timer = setTimeout(() => this.flush(), this.#delay);
	}

	/** Seeks now if a target is pending. */
	flush() {
		clearTimeout(this.#timer);
		const target = this.#target;
		this.#target = null;
		this.#repeats = 0;
		if (target !== null) {
			this.#onChange(null);
			this.#commit(target);
		}
	}

	cancel() {
		clearTimeout(this.#timer);
		this.#target = null;
		this.#repeats = 0;
		this.#onChange(null);
	}
}
