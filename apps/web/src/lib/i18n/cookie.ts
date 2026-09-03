// Client-side UI-language cookie helpers, mirroring `$lib/auth/cookie.ts`'s
// session cookie pattern. Kept separate from the session cookie because it
// answers a different question with a different lifetime: it's what lets a
// page with no signed-in user yet — the rest of the setup wizard after its
// own language step, or the login screen right after finishing setup —
// still render in the language just chosen, before there's any account row
// to read a saved preference from. Once signed in, the account's own
// `ui_language` (from `/api/auth/me`) is authoritative and wins over this
// cookie — see `+layout.ts` — this cookie only fills the gap before that
// exists, or on a browser an account has never signed into.

import { isSupportedLocale, type Locale } from './translations';

export const LOCALE_COOKIE_NAME = 'lisek_locale';

// Long-lived: a UI language choice isn't tied to a session and shouldn't
// need re-picking every time a session cookie happens to expire or get
// cleared by a sign-out.
const LOCALE_COOKIE_MAX_AGE_SECONDS = 60 * 60 * 24 * 365;

export function setLocaleCookie(locale: Locale): void {
	const secure = location.protocol === 'https:' ? '; Secure' : '';
	document.cookie = `${LOCALE_COOKIE_NAME}=${locale}; Path=/; Max-Age=${LOCALE_COOKIE_MAX_AGE_SECONDS}; SameSite=Lax${secure}`;
}

/**
 * Validates a raw cookie value (or `undefined`, if absent) into a `Locale`
 * or `null`. Pure and DOM-free on purpose, unlike `setLocaleCookie` above,
 * so `+layout.server.ts` can call it with whatever
 * `event.cookies.get(LOCALE_COOKIE_NAME)` returned — an absent or corrupted
 * cookie just means "no preference yet", left for the caller to default.
 */
export function parseLocaleCookie(value: string | undefined): Locale | null {
	return value && isSupportedLocale(value) ? value : null;
}
