import { describe, it, expect, vi, afterEach } from 'vitest';
import { fetchHomeFeed, imageUrl } from './api';

describe('imageUrl', () => {
	it('appends the token as a query param', () => {
		expect(imageUrl('/api/media/abc/poster', 'tok123')).toBe('/api/media/abc/poster?token=tok123');
	});

	it('encodes characters a token could contain', () => {
		expect(imageUrl('/api/media/abc/poster', 'a+b/c')).toBe(
			'/api/media/abc/poster?token=a%2Bb%2Fc'
		);
	});
});

describe('fetchHomeFeed', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('sends the auth header and returns the parsed feed', async () => {
		const feed = {
			hero: null,
			recently_added: [],
			top10: [],
			recommended: [],
			popular: []
		};
		const fetchMock = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			json: () => Promise.resolve(feed)
		});
		vi.stubGlobal('fetch', fetchMock);

		const result = await fetchHomeFeed('tok123');

		expect(result).toEqual(feed);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/home');
		expect(options.headers.Authorization).toBe('LisekMovie Token=tok123');
	});

	it('throws on a non-ok response', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn().mockResolvedValue({ ok: false, status: 401, json: () => Promise.resolve({}) })
		);

		await expect(fetchHomeFeed('tok123')).rejects.toThrow();
	});
});
