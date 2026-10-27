// Browser errors to the server log (Faza 17): `POST /api/logs/client`,
// stored with the tag [FRONTEND] and shown in Admin → Logs. Installed once
// by `hooks.client.ts` (uncaught errors, rejected promises) and called by
// SvelteKit's `handleError`.
//
// It never throws and never reports its own failures (no loops), sends the
// same message at most once every 30 s and at most 10 reports a minute.

import { buildAuthHeader } from '$lib/auth/header';
import { getSessionCookieClientSide } from '$lib/auth/cookie';

export const CLIENT_LOG_ENDPOINT = '/api/logs/client';

export interface ClientReport {
	level: 'error' | 'warn' | 'info';
	kind: string;
	message: string;
	stack?: string;
	url?: string;
	source?: string;
}

const REPEAT_WINDOW_MS = 30_000;
const MAX_PER_MINUTE = 10;

const lastSent = new Map<string, number>();
let minuteStart = 0;
let sentThisMinute = 0;

/** Clears the throttling state (tests). */
export function resetClientLog(): void {
	lastSent.clear();
	minuteStart = 0;
	sentThisMinute = 0;
}

/** Whether a report may go out now (and counts it). */
export function shouldSend(report: ClientReport, now = Date.now()): boolean {
	const key = `${report.kind}|${report.message}`;
	const previous = lastSent.get(key);
	if (previous !== undefined && now - previous < REPEAT_WINDOW_MS) return false;
	if (now - minuteStart >= 60_000) {
		minuteStart = now;
		sentThisMinute = 0;
	}
	if (sentThisMinute >= MAX_PER_MINUTE) return false;
	sentThisMinute += 1;
	lastSent.set(key, now);
	if (lastSent.size > 200) lastSent.delete(lastSent.keys().next().value as string);
	return true;
}

/** Message and stack of anything thrown. */
export function describeError(error: unknown): { message: string; stack?: string } {
	if (error instanceof Error) {
		return { message: `${error.name}: ${error.message}`, stack: error.stack };
	}
	if (typeof error === 'string') return { message: error };
	try {
		return { message: JSON.stringify(error) ?? String(error) };
	} catch {
		return { message: String(error) };
	}
}

export function reportClientError(report: ClientReport, fetchImpl: typeof fetch = fetch): void {
	try {
		if (!report.message.trim() || !shouldSend(report)) return;
		let token: string | null = null;
		try {
			token = getSessionCookieClientSide();
		} catch {
			// No document (tests, a worker): anonymous.
		}
		const body: ClientReport = {
			...report,
			message: report.message.slice(0, 2000),
			stack: report.stack?.slice(0, 8000),
			url: report.url ?? (typeof location !== 'undefined' ? location.href : undefined)
		};
		void fetchImpl(CLIENT_LOG_ENDPOINT, {
			method: 'POST',
			headers: {
				...(token ? buildAuthHeader(token) : {}),
				'Content-Type': 'application/json'
			},
			body: JSON.stringify(body),
			keepalive: true
		}).catch(() => {});
	} catch {
		// Reporting must never break the page.
	}
}

/** `window.onerror` and unhandled promise rejections. */
export function installErrorReporting(target: Window = window): () => void {
	const onError = (event: ErrorEvent) => {
		const { message, stack } = describeError(event.error ?? event.message);
		reportClientError({
			level: 'error',
			kind: 'error',
			message: event.message || message,
			stack,
			source: event.filename ? `${event.filename}:${event.lineno}:${event.colno}` : undefined
		});
	};
	const onRejection = (event: PromiseRejectionEvent) => {
		const { message, stack } = describeError(event.reason);
		// A failed report must not report itself.
		if (stack?.includes(CLIENT_LOG_ENDPOINT) || message.includes(CLIENT_LOG_ENDPOINT)) return;
		reportClientError({ level: 'error', kind: 'unhandledrejection', message, stack });
	};
	target.addEventListener('error', onError);
	target.addEventListener('unhandledrejection', onRejection);
	return () => {
		target.removeEventListener('error', onError);
		target.removeEventListener('unhandledrejection', onRejection);
	};
}
