// Client-side session cookie helpers. The cookie holds nothing but the
// opaque session token, and is deliberately NOT HttpOnly: both this module
// (client-side fetches, after hydration) and `+layout.server.ts` (server-side
// rendering) need to read it, to build the same `Authorization: LisekMovie
// Token=...` header the Rust backend actually validates — see the backend's
// `lisek-server/src/auth.rs` module doc comment for the full rationale,
// including the accepted tradeoff (a script-readable cookie is exposed to
// XSS the same way `localStorage` would be; acceptable for a self-hosted,
// single-tenant home server, revisit before ever exposing this beyond that).

export const SESSION_COOKIE_NAME = 'lisek_session';

// Mirrors `lisek_auth::SESSION_TTL` on the backend (30 days, fixed, no
// sliding renewal) — the cookie shouldn't outlive the session it holds, or
// linger meaninglessly after it.
const SESSION_COOKIE_MAX_AGE_SECONDS = 60 * 60 * 24 * 30;

export function setSessionCookie(token: string): void {
	const secure = location.protocol === 'https:' ? '; Secure' : '';
	document.cookie = `${SESSION_COOKIE_NAME}=${encodeURIComponent(token)}; Path=/; Max-Age=${SESSION_COOKIE_MAX_AGE_SECONDS}; SameSite=Lax${secure}`;
}

export function clearSessionCookie(): void {
	document.cookie = `${SESSION_COOKIE_NAME}=; Path=/; Max-Age=0; SameSite=Lax`;
}

/**
 * Reads the session cookie from `document.cookie` — browser-only. Server-side
 * code (load functions, hooks) must use SvelteKit's `event.cookies.get()`
 * instead; this throws if called with no `document` so a misuse fails loudly
 * rather than silently returning "not logged in" during SSR.
 */
export function getSessionCookieClientSide(): string | null {
	const match = document.cookie
		.split('; ')
		.find((entry) => entry.startsWith(`${SESSION_COOKIE_NAME}=`));
	return match ? decodeURIComponent(match.slice(SESSION_COOKIE_NAME.length + 1)) : null;
}
