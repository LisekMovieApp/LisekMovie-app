// Faza 25: named API keys for scripts and automations
// (`/api/admin/api-keys` on the backend). A key is shown once, when it is
// created; the list shows names, a short prefix, when each key was made
// and when it was last used.
import { buildAuthHeader } from '$lib/auth';
import { assertOk } from './api';

export interface ApiKey {
	id: string;
	name: string;
	/** The key's first characters (`lsk_AbC1`). */
	prefix: string;
	created_at: string;
	last_used_at: string | null;
}

export interface NewApiKey extends ApiKey {
	/** The whole key — shown once. */
	key: string;
}

/** What a key can read today (an API path, not translated). */
export const SUMMARY_ENDPOINT = 'GET /api/stats/summary?period=daily|weekly';

/** The header a script sends its key in. */
export const API_KEY_HEADER = 'X-Lisek-Api-Key';

export async function fetchApiKeys(token: string): Promise<ApiKey[]> {
	const response = await fetch('/api/admin/api-keys', { headers: buildAuthHeader(token) });
	await assertOk(response);
	return (await response.json()) as ApiKey[];
}

export async function createApiKey(token: string, name: string): Promise<NewApiKey> {
	const response = await fetch('/api/admin/api-keys', {
		method: 'POST',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ name })
	});
	await assertOk(response);
	return (await response.json()) as NewApiKey;
}

export async function renameApiKey(token: string, id: string, name: string): Promise<void> {
	const response = await fetch(`/api/admin/api-keys/${encodeURIComponent(id)}`, {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ name })
	});
	await assertOk(response);
}

export async function deleteApiKey(token: string, id: string): Promise<void> {
	const response = await fetch(`/api/admin/api-keys/${encodeURIComponent(id)}`, {
		method: 'DELETE',
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
}

/** A key name as the server accepts it: trimmed, 1–100 characters. */
export function validKeyName(name: string): boolean {
	const value = name.trim();
	return value.length > 0 && [...value].length <= 100;
}

/** A ready `curl` line for the bots' summary with `key`. */
export function curlExample(origin: string, key: string): string {
	return `curl -H "${API_KEY_HEADER}: ${key}" "${origin}/api/stats/summary?period=weekly&format=discord"`;
}
