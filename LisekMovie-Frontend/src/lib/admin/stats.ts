/**
 * Watch statistics dashboard helpers (Faza 13): the filter → query string,
 * date ranges, durations in the reports' style, and the labels of a
 * session. Pure, so they are tested without a browser.
 */
import type { components } from '$lib/api/schema';

export type Dashboard = components['schemas']['Dashboard'];
export type SessionRow = components['schemas']['SessionRow'];
export type TitleRank = components['schemas']['TitleRank'];
export type Slice = components['schemas']['Slice'];

export const PLATFORMS = [
	'windows',
	'macos',
	'linux',
	'android',
	'ios',
	'smart_tv',
	'other',
	'unknown'
] as const;
export const DEVICE_TYPES = ['desktop', 'mobile', 'tablet', 'tv', 'unknown'] as const;
export const CONTENT_TYPES = ['movie', 'series'] as const;

export interface StatsFilters {
	/** `YYYY-MM-DD` in the viewer's time zone, inclusive. */
	from: string;
	/** `YYYY-MM-DD`, inclusive. */
	to: string;
	userId: string;
	platform: string;
	deviceType: string;
	content: string;
	/** A film, episode or series id (from a ranking click). */
	item: string;
}

/** `YYYY-MM-DD` of a local date. */
export function dateInput(date: Date): string {
	const pad = (n: number) => String(n).padStart(2, '0');
	return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

/** The last `days` days, today included. */
export function lastDays(days: number, today = new Date()): { from: string; to: string } {
	const from = new Date(today);
	from.setDate(from.getDate() - (days - 1));
	return { from: dateInput(from), to: dateInput(today) };
}

/** Local midnight of a `YYYY-MM-DD` value, or `null` when malformed. */
function localMidnight(value: string): Date | null {
	const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
	if (!match) return null;
	const date = new Date(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
	return Number.isNaN(date.getTime()) ? null : date;
}

/**
 * The dashboard's query string: whole local days from `from` to `to`
 * (inclusive), the viewer's time zone for daily totals, and only the
 * filters that are set. `null` when the range is invalid.
 */
export function dashboardQuery(
	f: StatsFilters,
	timeZone: string,
	extra: Record<string, string> = {}
): string | null {
	const start = localMidnight(f.from);
	const endDay = localMidnight(f.to);
	if (!start || !endDay || endDay < start) return null;
	const end = new Date(endDay);
	end.setDate(end.getDate() + 1);
	const params = new URLSearchParams({
		from: start.toISOString(),
		to: end.toISOString(),
		tz: timeZone
	});
	if (f.userId) params.set('user_id', f.userId);
	if (f.platform) params.set('platform', f.platform);
	if (f.deviceType) params.set('device_type', f.deviceType);
	if (f.content) params.set('content', f.content);
	if (f.item) params.set('item', f.item);
	for (const [key, value] of Object.entries(extra)) params.set(key, value);
	return params.toString();
}

/** "1h 38m", "8m 48s", "45s" — the same style as the bot reports. */
export function humanDuration(seconds: number): string {
	const total = Number.isFinite(seconds) && seconds > 0 ? Math.round(seconds) : 0;
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	if (h > 0) return `${h}h ${m}m`;
	if (m > 0) return `${m}m ${s}s`;
	return `${s}s`;
}

/** How far into the title a session got, 0..1 (`null` without a length). */
export function progressOf(
	row: Pick<SessionRow, 'position_seconds' | 'duration_seconds'>
): number | null {
	if (!(row.duration_seconds > 0)) return null;
	return Math.min(1, Math.max(0, row.position_seconds / row.duration_seconds));
}

/** Share of `part` in `whole` as a whole percentage (0 when empty). */
export function percent(part: number, whole: number): number {
	return whole > 0 ? Math.round((part / whole) * 100) : 0;
}
