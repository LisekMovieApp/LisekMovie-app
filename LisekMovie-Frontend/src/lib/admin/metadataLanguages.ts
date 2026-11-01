// Faza 21: the metadata language order (`/api/admin/metadata/languages`).
// The scanner fetches a title's text in the first language; an empty
// synopsis or title is fetched again in the next one. Entries are language
// codes plus `original` (the production's own language) and `any`.
// Pure list operations live here so they are unit-tested without Svelte.

import { buildAuthHeader } from '$lib/auth';
import { assertOk } from './api';

export const ORIGINAL = 'original';
export const ANY = 'any';

/** What the language picker offers (names come from the dictionaries,
 * `admin.metadata_languages.lang_<code>`). */
export const LANGUAGE_CHOICES = [
	'pl',
	'en',
	'de',
	'fr',
	'es',
	'it',
	'pt',
	'cs',
	'uk',
	'ru',
	'ja',
	'ko',
	'zh',
	ORIGINAL,
	ANY
] as const;

export const MAX_LANGUAGES = 12;

export interface MetadataLanguages {
	/** As saved; empty when the default order is used. */
	languages: string[];
	/** The order the scanner uses now. */
	effective: string[];
}

export async function fetchMetadataLanguages(token: string): Promise<MetadataLanguages> {
	const response = await fetch('/api/admin/metadata/languages', {
		headers: buildAuthHeader(token)
	});
	await assertOk(response);
	return (await response.json()) as MetadataLanguages;
}

/** Saves `languages` (empty: back to the default order). */
export async function saveMetadataLanguages(
	token: string,
	languages: string[]
): Promise<MetadataLanguages> {
	const response = await fetch('/api/admin/metadata/languages', {
		method: 'PUT',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
		body: JSON.stringify({ languages })
	});
	await assertOk(response);
	return (await response.json()) as MetadataLanguages;
}

/** `list` with the entry at `index` moved by `delta` (-1 up, +1 down). */
export function moveEntry(list: string[], index: number, delta: number): string[] {
	const target = index + delta;
	if (index < 0 || index >= list.length || target < 0 || target >= list.length) return list;
	const next = [...list];
	[next[index], next[target]] = [next[target], next[index]];
	return next;
}

/** `list` with `code` added at the end (not twice, at most MAX_LANGUAGES). */
export function addEntry(list: string[], code: string): string[] {
	if (!code || list.includes(code) || list.length >= MAX_LANGUAGES) return list;
	return [...list, code];
}

/** The picker's choices not in `list` yet. */
export function remainingChoices(list: string[]): string[] {
	return LANGUAGE_CHOICES.filter((code) => !list.includes(code));
}

/** Translation key of an entry's name. */
export function languageLabelKey(code: string): string {
	return `admin.metadata_languages.lang_${code}`;
}
