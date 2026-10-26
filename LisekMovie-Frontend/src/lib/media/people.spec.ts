import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	BIOGRAPHY_PREVIEW_CHARS,
	creditCaption,
	fetchItemPeople,
	fetchPerson,
	formatLifeDate,
	initials,
	isLongBiography
} from './people';
import { MediaNotFoundError } from './api';

afterEach(() => {
	vi.unstubAllGlobals();
});

describe('people', () => {
	it('captions actors with their character and crew with their role', () => {
		expect(creditCaption({ role: 'Actor', character: 'Han Solo' })).toEqual({
			text: 'Han Solo',
			roleKey: null
		});
		expect(creditCaption({ role: 'Actor', character: null })).toEqual({
			text: null,
			roleKey: null
		});
		expect(creditCaption({ role: 'Director', character: null })).toEqual({
			text: null,
			roleKey: 'people.role.Director'
		});
	});

	it('formats calendar dates without shifting the day', () => {
		expect(formatLifeDate('1942-07-13', 'pl')).toBe('13 lipca 1942');
		expect(formatLifeDate('1942-07-13', 'en')).toBe('July 13, 1942');
		expect(formatLifeDate('not a date', 'en')).toBe('not a date');
	});

	it('collapses only long biographies', () => {
		expect(isLongBiography(null)).toBe(false);
		expect(isLongBiography('x'.repeat(BIOGRAPHY_PREVIEW_CHARS))).toBe(false);
		expect(isLongBiography('x'.repeat(BIOGRAPHY_PREVIEW_CHARS + 1))).toBe(true);
	});

	it('builds initials', () => {
		expect(initials('Harrison Ford')).toBe('HF');
		expect(initials(' Zendaya ')).toBe('Z');
		expect(initials('Jean-Claude Van Damme')).toBe('JD');
		expect(initials('')).toBe('');
	});

	it('loads credits and reports a missing person', async () => {
		const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
			void init;
			if (url === '/api/media/m1/people') {
				return new Response(
					JSON.stringify({ items: [{ person_id: 'p1', name: 'A', role: 'Actor' }] })
				);
			}
			return new Response('', { status: 404 });
		});
		vi.stubGlobal('fetch', fetchMock);
		const credits = await fetchItemPeople('tok', 'm1');
		expect(credits.map((c) => c.person_id)).toEqual(['p1']);
		expect(fetchMock.mock.calls[0][1]).toMatchObject({
			headers: { Authorization: expect.stringContaining('tok') }
		});
		await expect(fetchPerson('tok', 'missing')).rejects.toBeInstanceOf(MediaNotFoundError);
	});
});
