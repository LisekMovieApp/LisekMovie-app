/**
 * Builds the `Authorization` header value the backend's `AuthUser` extractor
 * expects. Used identically from server-side load functions (with the token
 * read via `event.cookies.get`) and client-side fetches (with the token from
 * `getSessionCookieClientSide`) — one function, one header shape, everywhere
 * a request needs to prove who's logged in.
 */
export function buildAuthHeader(token: string): Record<string, string> {
	return { Authorization: `LisekMovie Token=${token}` };
}
