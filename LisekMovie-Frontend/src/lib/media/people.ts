// Cast and crew (Faza 8): an item's credits (`GET /api/media/{id}/people`)
// and a person's page (`GET /api/people/{id}`). Photos come from
// `/api/people/{id}/image`, loaded through `imageUrl` like posters.

import type { components } from '$lib/api/schema';
import { buildAuthHeader } from '$lib/auth';
import { MediaNotFoundError, type MediaItem } from './api';

export type Credit = components['schemas']['CreditResponse'];
/** A title crediting the person: the usual card plus what they did on it. */
export type FilmographyItem = Omit<components['schemas']['FilmographyItemResponse'], 'item'> & {
	item: MediaItem;
};
export type Person = Omit<components['schemas']['PersonResponse'], 'filmography'> & {
	filmography: FilmographyItem[];
};

/** The detail page's cast row: directors, writers, then the cast. */
export async function fetchItemPeople(token: string, itemId: string): Promise<Credit[]> {
	const url = `/api/media/${encodeURIComponent(itemId)}/people`;
	const response = await fetch(url, { headers: buildAuthHeader(token) });
	if (!response.ok) throw new Error(`GET ${url} failed with status ${response.status}`);
	return ((await response.json()) as { items: Credit[] }).items;
}

/** `/person/[id]`. Throws `MediaNotFoundError` for a 404 (no such person,
 * or none of their titles is visible to this account). */
export async function fetchPerson(token: string, personId: string): Promise<Person> {
	const url = `/api/people/${encodeURIComponent(personId)}`;
	const response = await fetch(url, { headers: buildAuthHeader(token) });
	if (response.status === 404) throw new MediaNotFoundError();
	if (!response.ok) throw new Error(`GET ${url} failed with status ${response.status}`);
	return (await response.json()) as Person;
}

/** The line under a name on a cast card: the character for an actor,
 * otherwise the role's translation key (`people.role.Director`). */
export function creditCaption(credit: Pick<Credit, 'role' | 'character'>): {
	text: string | null;
	roleKey: string | null;
} {
	if (credit.role === 'Actor') return { text: credit.character ?? null, roleKey: null };
	return { text: null, roleKey: `people.role.${credit.role}` };
}

/** "13 lipca 1942" — a `YYYY-MM-DD` date in the UI language. The date is
 * a calendar day, not an instant: formatted in UTC so no time zone moves
 * it to the day before. */
export function formatLifeDate(date: string, locale: string): string {
	const parsed = new Date(`${date}T00:00:00Z`);
	if (Number.isNaN(parsed.getTime())) return date;
	return new Intl.DateTimeFormat(locale, {
		day: 'numeric',
		month: 'long',
		year: 'numeric',
		timeZone: 'UTC'
	}).format(parsed);
}

/** Biographies longer than this start collapsed behind "Show more". */
export const BIOGRAPHY_PREVIEW_CHARS = 600;

export function isLongBiography(biography: string | null | undefined): boolean {
	return (biography?.length ?? 0) > BIOGRAPHY_PREVIEW_CHARS;
}

/** Up to two letters for a person without a photo: "Harrison Ford" → "HF". */
export function initials(name: string): string {
	const words = name.trim().split(/\s+/).filter(Boolean);
	if (words.length === 0) return '';
	const first = words[0][0] ?? '';
	const last = words.length > 1 ? (words[words.length - 1][0] ?? '') : '';
	return (first + last).toUpperCase();
}
