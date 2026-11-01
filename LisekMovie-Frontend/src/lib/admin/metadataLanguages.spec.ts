import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	addEntry,
	fetchMetadataLanguages,
	languageLabelKey,
	MAX_LANGUAGES,
	moveEntry,
	remainingChoices,
	saveMetadataLanguages
} from './metadataLanguages';

afterEach(() => {
	vi.unstubAllGlobals();
});

function stubFetch(body: unknown, ok = true) {
	const fetchMock = vi.fn().mockResolvedValue({
		ok,
		status: ok ? 200 : 400,
		json: () => Promise.resolve(body),
		clone() {
			return this;
		}
	});
	vi.stubGlobal('fetch', fetchMock);
	return fetchMock;
}

describe('metadata language order', () => {
	it('moves entries up and down within bounds', () => {
		expect(moveEntry(['pl', 'en', 'original'], 2, -1)).toEqual(['pl', 'original', 'en']);
		expect(moveEntry(['pl', 'en'], 0, 1)).toEqual(['en', 'pl']);
		const list = ['pl', 'en'];
		expect(moveEntry(list, 0, -1)).toBe(list);
		expect(moveEntry(list, 1, 1)).toBe(list);
	});

	it('adds an entry once and stops at the limit', () => {
		expect(addEntry(['pl'], 'en')).toEqual(['pl', 'en']);
		expect(addEntry(['pl'], 'pl')).toEqual(['pl']);
		const full = Array.from({ length: MAX_LANGUAGES }, (_, n) => `x${n}`);
		expect(addEntry(full, 'pl')).toBe(full);
	});

	it('offers only what is not listed', () => {
		const left = remainingChoices(['pl', 'original']);
		expect(left).toContain('en');
		expect(left).not.toContain('pl');
		expect(left).not.toContain('original');
		expect(languageLabelKey('any')).toBe('admin.metadata_languages.lang_any');
	});

	it('loads and saves through the admin endpoint', async () => {
		const answer = { languages: ['pl', 'en'], effective: ['pl', 'en'] };
		const fetchMock = stubFetch(answer);
		expect(await fetchMetadataLanguages('tok')).toEqual(answer);
		expect(fetchMock.mock.calls[0][0]).toBe('/api/admin/metadata/languages');
		await saveMetadataLanguages('tok', ['pl', 'en']);
		const [, init] = fetchMock.mock.calls[1];
		expect(init.method).toBe('PUT');
		expect(JSON.parse(init.body)).toEqual({ languages: ['pl', 'en'] });
	});

	it('rejects a refused order', async () => {
		stubFetch({ error_code: 'invalid_input' }, false);
		await expect(saveMetadataLanguages('tok', ['polski'])).rejects.toMatchObject({
			status: 400,
			code: 'invalid_input'
		});
	});
});
