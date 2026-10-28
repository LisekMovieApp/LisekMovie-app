// Faza 19: the signed-in account's own profile (avatar, UI language) and
// the presence keep-alive behind the admin user list's online dots.
import { buildAuthHeader } from '$lib/auth';
import { accountRequest } from './api';

/** Largest avatar upload the server accepts (`profile::MAX_AVATAR_BYTES`). */
export const MAX_AVATAR_BYTES = 10 * 1024 * 1024;
export const AVATAR_TYPES = ['image/jpeg', 'image/png', 'image/webp', 'image/gif'];
/** How often an open, visible tab tells the server it is still there. */
export const HEARTBEAT_MS = 60_000;

/** An avatar URL usable in `<img src>`: image tags cannot send headers,
 * so the session goes into the query string (the server allows it only
 * for image paths). */
export function avatarSrc(
	url: string | null | undefined,
	token: string | null | undefined
): string | null {
	if (!url) return null;
	if (!token) return url;
	return `${url}${url.includes('?') ? '&' : '?'}token=${encodeURIComponent(token)}`;
}

/** Client-side check before uploading: an error key, or null when fine. */
export function avatarFileError(file: Pick<File, 'type' | 'size'>): string | null {
	if (!AVATAR_TYPES.includes(file.type)) return 'profile.errors.avatar_invalid_image';
	if (file.size > MAX_AVATAR_BYTES) return 'profile.errors.avatar_too_large';
	if (file.size === 0) return 'profile.errors.avatar_empty';
	return null;
}

export class ProfileError extends Error {}

/** Uploads the picture as the raw request body; returns the new avatar URL. */
export async function uploadAvatar(token: string, file: Blob): Promise<string | null> {
	const response = await fetch('/api/users/me/avatar', {
		method: 'POST',
		cache: 'no-store',
		headers: { ...buildAuthHeader(token), 'Content-Type': file.type || 'application/octet-stream' },
		body: file
	});
	const data = await response.json().catch(() => null);
	if (!response.ok)
		throw new ProfileError(
			data?.error_code ?? (response.status === 413 ? 'avatar_too_large' : 'internal_error')
		);
	return data?.avatar_url ?? null;
}

export async function deleteAvatar(token: string): Promise<void> {
	await accountRequest(token, '/api/users/me/avatar', 'DELETE');
}

export async function setLanguage(token: string, language: string): Promise<void> {
	await accountRequest(token, '/api/users/me/language', 'PUT', { ui_language: language });
}

const KNOWN = new Set([
	'avatar_invalid_image',
	'avatar_too_large',
	'avatar_empty',
	'invalid_language'
]);

/** Translation key for a failed profile request. */
export function profileErrorKey(error: unknown): string {
	const code = error instanceof Error ? error.message : '';
	return `profile.errors.${KNOWN.has(code) ? code : 'operation_failed'}`;
}

/** Keeps the account marked as online while a tab is visible. Returns the
 * cleanup function. */
export function startHeartbeat(getToken: () => string | null): () => void {
	const beat = () => {
		const token = getToken();
		if (!token || document.visibilityState !== 'visible') return;
		void fetch('/api/users/me/heartbeat', {
			method: 'POST',
			headers: buildAuthHeader(token)
		}).catch(() => {});
	};
	const timer = setInterval(beat, HEARTBEAT_MS);
	const onVisible = () => {
		if (document.visibilityState === 'visible') beat();
	};
	document.addEventListener('visibilitychange', onVisible);
	return () => {
		clearInterval(timer);
		document.removeEventListener('visibilitychange', onVisible);
	};
}

/** "3 minutes ago" in the UI language. */
export function relativeTime(iso: string, locale: string, now = Date.now()): string {
	const seconds = Math.round((new Date(iso).getTime() - now) / 1000);
	const format = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
	const steps: [Intl.RelativeTimeFormatUnit, number][] = [
		['second', 60],
		['minute', 60],
		['hour', 24],
		['day', 30],
		['month', 12],
		['year', Infinity]
	];
	let value = seconds;
	for (const [unit, size] of steps) {
		if (Math.abs(value) < size) return format.format(value, unit);
		value = Math.round(value / size);
	}
	return format.format(value, 'year');
}
