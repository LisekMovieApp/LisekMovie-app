// Reactive current-locale store, using a Svelte 5 rune instead of Svelte 4's
// `writable` store — this is a `.svelte.ts` module specifically so `$state`
// is allowed here.
//
// SSR note: this is a module-level singleton, which is normally unsafe for
// per-request state on a server (one request's value could leak into
// another's rendered HTML) if it's mutated from inside an async `load`
// function — after an `await`, a concurrent request's own `load` could
// interleave and stomp this one's value before it's read. The root layout
// (`+layout.svelte`) resolves the locale itself, from `data` a `load`
// function already computed, and calls `setLocale` from its own top-level
// script — that runs as part of Svelte's SSR render of the component tree,
// which is a single synchronous call with no `await` inside it, so no other
// request's render can interleave between that assignment and this whole
// request finishing. The rule going forward: only ever mutate
// `localeState` synchronously — from a browser event handler, or from a
// component's own top-level/`$effect` code — never from inside an async
// `load` function.

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
