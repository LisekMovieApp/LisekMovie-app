// Server-only load — this is the one place allowed to read cookies
// directly (SvelteKit's `cookies` API isn't available to universal
// `+layout.ts` load functions). Passes the raw token down as page data;
// `+layout.ts` is what actually calls /api/auth/me and decides on a
// redirect, so the two files together are what "sign-in required" means.
//
// Also reads the UI-language cookie the same way, for the same reason: it's
// the only pre-account signal `+layout.ts` has for what language to render
// in (see `$lib/i18n/cookie.ts` and `+layout.ts` for how it's combined with
// a signed-in account's saved `ui_language`).
import type { LayoutServerLoad } from './$types';
import { SESSION_COOKIE_NAME } from '$lib/auth';
import { LOCALE_COOKIE_NAME, parseLocaleCookie } from '$lib/i18n';

export const load: LayoutServerLoad = ({ cookies }) => {
	return {
		token: cookies.get(SESSION_COOKIE_NAME) ?? null,
		localeFromCookie: parseLocaleCookie(cookies.get(LOCALE_COOKIE_NAME))
	};
};
