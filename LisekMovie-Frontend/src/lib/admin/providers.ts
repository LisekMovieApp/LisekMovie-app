// Client-side calls for the Admin Panel's plugins tab (`/api/admin/providers/*`
// on the backend). Kept separate from the page/modal components so the
// request shapes are unit-testable without rendering Svelte — mirrors this
// project's existing split between pure logic and DOM-touching components
// (see `carouselMath.ts`, `cookie.ts`).

import { buildAuthHeader } from '$lib/auth';
import { assertOk, AdminApiError } from './api';

// Re-exported for existing callers/tests that import `AdminApiError` from
// this module specifically — the class itself now lives in `./api` so
// `libraries.ts` can throw and catch the same one.
export { AdminApiError };

export type KeyRequirement = 'required' | 'optional' | 'none';
export type ProviderCoverage = 'movies' | 'series' | 'anime' | 'music_videos' | 'music';
export type ProviderStage = 'full' | 'basic' | 'skeleton';

export interface ProviderConfig {
	provider_key: string;
	display_name: string;
	enabled: boolean;
	priority: number;
	/** A key is stored, or the provider works without one. */
	is_configured: boolean;
	/** A key is stored (never the key itself). */
	key_stored: boolean;
	key_requirement: KeyRequirement;
	covers: ProviderCoverage[];
	stage: ProviderStage;
	website?: string | null;
}

/** Providers that can be asked right now: enabled, with a key when one is needed. */
export function usableProviders(providers: ProviderConfig[]): ProviderConfig[] {
	return providers.filter((p) => p.enabled && p.is_configured);
}

export async function fetchProviders(token: string): Promise<ProviderConfig[]> {
	const response = await fetch('/api/admin/providers', { headers: buildAuthHeader(token) });
	await assertOk(response);
	return (await response.json()) as ProviderConfig[];
}

export async function updateProvider(
	token: string,
	providerKey: string,
	enabled: boolean,
	priority: number
): Promise<void> {
	const response = await fetch(`/api/admin/providers/${encodeURIComponent(providerKey)}`, {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ enabled, priority })
	});
	await assertOk(response);
}

export async function setProviderApiKey(
	token: string,
	providerKey: string,
	apiKey: string
): Promise<void> {
	const response = await fetch(`/api/admin/providers/${encodeURIComponent(providerKey)}/key`, {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ api_key: apiKey })
	});
	await assertOk(response);
}

export async function clearProviderApiKey(token: string, providerKey: string): Promise<void> {
	const response = await fetch(`/api/admin/providers/${encodeURIComponent(providerKey)}/key`, {
		method: 'DELETE',
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
}
