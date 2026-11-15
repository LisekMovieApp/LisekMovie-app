import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	createApiKey,
	curlExample,
	deleteApiKey,
	fetchApiKeys,
	renameApiKey,
	validKeyName
} from './apiKeys';

afterEach(() => vi.unstubAllGlobals());

function stubFetch(body: unknown = null, status = 200) {
	const fetchMock = vi.fn().mockResolvedValue({
		ok: status < 400,
		status,
		json: () => Promise.resolve(body),
		clone() {
			return this;
		}
	});
	vi.stubGlobal('fetch', fetchMock);
	return fetchMock;
}

describe('API keys (Faza 25)', () => {
	it('lists, creates, renames and revokes keys', async () => {
		const fetchMock = stubFetch([]);
		await fetchApiKeys('tok');
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/api-keys');
		await createApiKey('tok', 'Mój Bot');
		expect(fetchMock.mock.calls[1][1].method).toBe('POST');
		expect(JSON.parse(fetchMock.mock.calls[1][1].body)).toEqual({ name: 'Mój Bot', scopes: ['stats'] });
		await renameApiKey('tok', 'k1', 'HA');
		expect(fetchMock.mock.calls[2][0]).toBe('/api/admin/api-keys/k1');
		expect(fetchMock.mock.calls[2][1].method).toBe('PUT');
		await deleteApiKey('tok', 'k1');
		expect(fetchMock.mock.calls[3][1].method).toBe('DELETE');
	});

	it('checks names and builds the curl example', () => {
		expect(validKeyName('  Discord ')).toBe(true);
		expect(validKeyName('   ')).toBe(false);
		expect(validKeyName('x'.repeat(101))).toBe(false);
		expect(curlExample('https://lisek.local', 'lsk_abc')).toBe(
			'curl -H "X-Lisek-Api-Key: lsk_abc" "https://lisek.local/api/stats/summary?period=weekly&format=discord"'
		);
	});
});
