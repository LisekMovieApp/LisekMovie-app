import { afterEach, describe, expect, it, vi } from 'vitest';
import { identifyItem, idHint, parseYear, searchProviders } from './identify';
import { formatSize, startDownload } from './download';

describe('fix match and downloads', () => {
	afterEach(() => vi.unstubAllGlobals());

	it('builds the provider search and identify requests', async () => {
		const fetchMock = vi.fn().mockResolvedValue({
			ok: true,
			status: 200,
			json: () => Promise.resolve({ results: [], failures: [] })
		});
		vi.stubGlobal('fetch', fetchMock);
		await searchProviders('t', 'series', ' Inazuma Eleven ', 2008, 'tvdb');
		expect(fetchMock.mock.calls[0][0]).toBe(
			'/api/providers/search?kind=series&query=Inazuma+Eleven&year=2008&provider=tvdb'
		);
		await identifyItem('t', 'abc', 'tmdb', '11');
		const [url, options] = fetchMock.mock.calls[1];
		expect(url).toBe('/api/media/abc/identify');
		expect(options.method).toBe('POST');
		expect(JSON.parse(options.body)).toEqual({ provider: 'tmdb', provider_id: '11' });
	});

	it('reads ids the way the server does', () => {
		expect(idHint('tt0425622')).toBe('imdb');
		expect(idHint(' TT0425622 ')).toBe('imdb');
		expect(idHint('60620')).toBe('number');
		expect(idHint('tvdb:60620')).toBe('prefixed');
		expect(idHint('imdb:tt0133093')).toBe('prefixed');
		expect(idHint('https://www.imdb.com/title/tt0425622/')).toBe('link');
		expect(idHint('https://www.themoviedb.org/movie/603-the-matrix')).toBe('link');
		expect(idHint('https://anilist.co/anime/21/ONE-PIECE')).toBe('link');
		expect(idHint('Star Wars')).toBeNull();
		expect(idHint('tt12')).toBeNull();
		expect(idHint('foo:12')).toBeNull();
		expect(idHint('')).toBeNull();
	});

	it('accepts only a four-digit year', () => {
		expect(parseYear('1977')).toBe(1977);
		expect(parseYear(' 77 ')).toBeNull();
		expect(parseYear('')).toBeNull();
	});

	it('formats sizes and starts a download through a link', () => {
		expect(formatSize(512)).toBe('512 B');
		expect(formatSize(1536 * 1024 * 1024)).toBe('1.5 GB');
		const click = vi.fn();
		const link = { click, remove: vi.fn(), href: '', download: '', rel: '' };
		const doc = {
			createElement: () => link,
			body: { appendChild: vi.fn() }
		} as unknown as Document;
		startDownload(
			{
				url: '/api/media/x/download?ticket=y',
				file_name: 'Film_1977.mkv',
				size_bytes: 1,
				expires_in: 1
			},
			doc
		);
		expect(link.href).toBe('/api/media/x/download?ticket=y');
		expect(link.download).toBe('Film_1977.mkv');
		expect(click).toHaveBeenCalled();
	});
});
