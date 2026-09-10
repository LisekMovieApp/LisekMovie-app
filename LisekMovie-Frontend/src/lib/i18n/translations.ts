// Static translation dictionaries and the pure lookup/interpolation logic.
// Kept separate from the reactive locale store (`locale.svelte.ts`) so this
// file has no Svelte compiler dependency and stays trivially unit-testable.
//
// Only English and Polish ship real copy for now. Czech is the project's
// third planned language (see the roadmap), but its UI strings haven't been
// written and reviewed yet — shipping a machine-translated stub would look
// like a finished, wrong translation rather than a missing one. Adding it
// later is just a new `cs.json` plus one import and an entry in `dictionaries`.

import en from './en.json';
import pl from './pl.json';

const dictionaries = { en, pl };
export type Locale = keyof typeof dictionaries;

export const SUPPORTED_LOCALES = Object.keys(dictionaries) as Locale[];
export const DEFAULT_LOCALE: Locale = 'en';


function lookup(dictionary: unknown, path: string[]): string | undefined {
	let node: unknown = dictionary;
	for (const segment of path) {
		if (typeof node !== 'object' || node === null || !(segment in node)) {
			return undefined;
		}
		node = (node as Record<string, unknown>)[segment];
	}
	return typeof node === 'string' ? node : undefined;
}

/**
 * Looks up `key` (dot-separated, e.g. `"setup.step_admin_title"`) in the
 * given locale's dictionary, falling back to `DEFAULT_LOCALE` and then to
 * the raw key itself so a missing translation is visible instead of blank.
 * `params` are substituted as `{name}` placeholders in the resolved string.
 */
export function translate(
	locale: Locale,
	key: string,
	params?: Record<string, string | number>
): string {
	const path = key.split('.');
	const value =
		lookup(dictionaries[locale], path) ?? lookup(dictionaries[DEFAULT_LOCALE], path) ?? key;

	if (!params) return value;

	return Object.entries(params).reduce(
		(text, [name, replacement]) => text.replaceAll(`{${name}}`, String(replacement)),
		value
	);
}

export function isSupportedLocale(value: string): value is Locale {
	return (SUPPORTED_LOCALES as string[]).includes(value);
}
