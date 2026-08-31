// Reactive current-locale store, using a Svelte 5 rune instead of Svelte 4's
// `writable` store — this is a `.svelte.ts` module specifically so `$state`
// is allowed here.
//
// SSR note: this is a module-level singleton, which is normally unsafe for
// per-request state on a server (one request's value could leak into
// another's rendered HTML). It's safe here only because nothing sets it
// during server-side rendering — `setLocale` is only ever called from
// browser event handlers. If a later phase adds server-side locale
// detection (e.g. from a saved user preference), it must NOT call
// `setLocale` from a `load` function; it should pass the locale down through
// page data instead.

import { DEFAULT_LOCALE, translate, type Locale } from './translations';

const localeState = $state<{ current: Locale }>({ current: DEFAULT_LOCALE });

export function setLocale(locale: Locale): void {
	localeState.current = locale;
}

export function getLocale(): Locale {
	return localeState.current;
}

/**
 * Translation helper for use inside `.svelte` components: `t('setup.next')`.
 * Reads the reactive locale, so any expression using it re-renders when
 * `setLocale` changes it.
 */
export function t(key: string, params?: Record<string, string | number>): string {
	return translate(localeState.current, key, params);
}
