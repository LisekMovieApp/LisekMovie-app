/** "12:05", "1:02:03"; negative or unknown times show as "0:00". */
export function formatTime(seconds: number): string {
	const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	const ss = String(s).padStart(2, '0');
	return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

/** End of the buffered range that contains `time` (or `time` itself). */
export function bufferedEnd(ranges: TimeRanges | null | undefined, time: number): number {
	if (!ranges) return time;
	for (let i = 0; i < ranges.length; i++) {
		if (ranges.start(i) <= time + 0.5 && ranges.end(i) >= time) return ranges.end(i);
	}
	return time;
}

/** Clamps a seek target into the playable range. */
export function clampTime(time: number, duration: number): number {
	if (!Number.isFinite(time)) return 0;
	const end = Number.isFinite(duration) && duration > 0 ? Math.max(0, duration - 0.5) : Infinity;
	return Math.min(Math.max(0, time), end);
}
