// Server-only load — this is the one place allowed to read the session
// cookie directly (SvelteKit's `cookies` API isn't available to universal
// `+layout.ts` load functions). Passes the raw token down as page data;
// `+layout.ts` is what actually calls /api/auth/me and decides on a
// redirect, so the two files together are what "sign-in required" means.
import type { LayoutServerLoad } from './$types';
import { SESSION_COOKIE_NAME } from '$lib/auth';

export const load: LayoutServerLoad = ({ cookies }) => {
	return { token: cookies.get(SESSION_COOKIE_NAME) ?? null };
};
