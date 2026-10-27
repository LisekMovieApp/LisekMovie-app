// Admin → Logs (Faza 17): the server's daily log files (`/api/admin/logs`).

import type { components } from '$lib/api/schema';
import { buildAuthHeader } from '$lib/auth/header';
import { api } from '$lib/playback/api';

export type LogFiles = components['schemas']['LogFilesResponse'];
/** The event's own fields are free-form (`item_id`, `err`, `stack`...). */
export type LogEntry = Omit<components['schemas']['LogEntry'], 'fields'> & {
	fields: Record<string, unknown>;
};
export type LogDay = Omit<components['schemas']['LogDayResponse'], 'entries'> & {
	entries: LogEntry[];
};

export const LOG_SOURCES = ['backend', 'frontend', 'ffmpeg', 'ffprobe'] as const;
export type LogSource = (typeof LOG_SOURCES)[number];
export const LOG_LEVELS = ['error', 'warn', 'info', 'debug'] as const;
export type LogLevel = (typeof LOG_LEVELS)[number];

export function fetchLogFiles(token: string): Promise<LogFiles> {
	return api<LogFiles>(token, '/api/admin/logs');
}

/** All sources / levels selected: no filter sent. */
export function logQuery(
	sources: readonly LogSource[],
	levels: readonly LogLevel[],
	limit: number
): string {
	const params = new URLSearchParams({ limit: String(limit) });
	if (sources.length > 0 && sources.length < LOG_SOURCES.length)
		params.set('source', sources.join(','));
	if (levels.length > 0 && levels.length < LOG_LEVELS.length) params.set('level', levels.join(','));
	return params.toString();
}

export function fetchLogDay(
	token: string,
	date: string,
	sources: readonly LogSource[],
	levels: readonly LogLevel[],
	limit = 3000
): Promise<LogDay> {
	return api<LogDay>(
		token,
		`/api/admin/logs/${encodeURIComponent(date)}?${logQuery(sources, levels, limit)}`
	);
}

/** Saves the raw day file (JSON lines). */
export async function downloadLogFile(token: string, date: string, doc: Document = document) {
	const response = await fetch(`/api/admin/logs/${encodeURIComponent(date)}/raw`, {
		headers: buildAuthHeader(token)
	});
	if (!response.ok) throw new Error(`HTTP ${response.status}`);
	const url = URL.createObjectURL(await response.blob());
	const link = doc.createElement('a');
	link.href = url;
	link.download = `lisekmovie.${date}.log`;
	doc.body.appendChild(link);
	link.click();
	link.remove();
	setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/** `12:04:05` in the viewer's time zone (the files are UTC). */
export function clockTime(timestamp: string): string {
	const date = new Date(timestamp);
	if (Number.isNaN(date.getTime())) return '';
	return date.toLocaleTimeString(undefined, { hour12: false });
}

/** The message without the `[FFMPEG]`-style tag the source column
 * already shows. */
export function displayMessage(entry: LogEntry): string {
	const tag = `[${entry.source.toUpperCase()}] `;
	return entry.message.startsWith(tag) ? entry.message.slice(tag.length) : entry.message;
}

/** The fields worth a second line (`item_id`, `err`, `stack`...). */
export function extraFields(entry: LogEntry): [string, string][] {
	return Object.entries(entry.fields ?? {})
		.filter(([, value]) => value !== null && value !== undefined && value !== '')
		.map(([key, value]) => [key, typeof value === 'string' ? value : JSON.stringify(value)]);
}

/** Case-insensitive text search over the message and the fields. */
export function matchesText(entry: LogEntry, text: string): boolean {
	const needle = text.trim().toLowerCase();
	if (!needle) return true;
	if (entry.message.toLowerCase().includes(needle)) return true;
	return extraFields(entry).some(([key, value]) =>
		`${key}=${value}`.toLowerCase().includes(needle)
	);
}

export function formatBytes(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
