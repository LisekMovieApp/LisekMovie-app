import { describe, it, expect, vi, afterEach } from 'vitest';
import {
	fetchProviders,
	updateProvider,
	setProviderApiKey,
	clearProviderApiKey,
	AdminApiError
} from './providers';

function stubFetch(response: { ok: boolean; status?: number; json?: () => unknown }) {
	const fetchMock = vi.fn().mockResolvedValue({
		ok: response.ok,
		status: response.status ?? (response.ok ? 200 : 500),
		json: response.json ?? (() => Promise.resolve(undefined))
	});
	vi.stubGlobal('fetch', fetchMock);
	return fetchMock;
}

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('fetchProviders', () => {
	it('sends the auth header and returns the parsed list', async () => {
		const providers = [
			{
				provider_key: 'tmdb',
				display_name: 'TMDB',
				enabled: true,
				priority: 0,
				is_configured: false
			}
		];
		const fetchMock = stubFetch({ ok: true, json: () => Promise.resolve(providers) });

		const result = await fetchProviders('tok123');

		expect(result).toEqual(providers);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/providers');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
	});

	it('throws AdminApiError with the status on a non-ok response', async () => {
		stubFetch({ ok: false, status: 403 });

		await expect(fetchProviders('tok123')).rejects.toMatchObject({
			status: 403
		});
		await expect(fetchProviders('tok123')).rejects.toBeInstanceOf(AdminApiError);
	});
});

describe('updateProvider', () => {
	it('PUTs enabled/priority as JSON to the provider-key path', async () => {
		const fetchMock = stubFetch({ ok: true });

		await updateProvider('tok123', 'tvdb', false, 3);

		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/providers/tvdb');
		expect(options.method).toBe('PUT');
		expect(JSON.parse(options.body)).toEqual({ enabled: false, priority: 3 });
	});

	it('URL-encodes a provider key with special characters', async () => {
		const fetchMock = stubFetch({ ok: true });

		await updateProvider('tok123', 'weird key/x', true, 0);

		const [url] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/providers/weird%20key%2Fx');
	});
});

describe('setProviderApiKey', () => {
	it('PUTs the api_key to the /key sub-path', async () => {
		const fetchMock = stubFetch({ ok: true });

		await setProviderApiKey('tok123', 'tmdb', 'secret-value');

		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/providers/tmdb/key');
		expect(options.method).toBe('PUT');
		expect(JSON.parse(options.body)).toEqual({ api_key: 'secret-value' });
	});
});

describe('clearProviderApiKey', () => {
	it('DELETEs the /key sub-path', async () => {
		const fetchMock = stubFetch({ ok: true });

		await clearProviderApiKey('tok123', 'tmdb');

		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/admin/providers/tmdb/key');
		expect(options.method).toBe('DELETE');
	});
});
