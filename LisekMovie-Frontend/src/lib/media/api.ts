// Client-side call for the home page's feed (`GET /api/home`) — the hero
// item plus the four rows, in one round trip. See `lisek-server/src/
// media.rs` for the exact response shape this mirrors.

import { buildAuthHeader } from '$lib/auth';

export interface MediaItem {
	id: string;
	item_type: 'movie' | 'episode';
	title: string;
	year: number | null;
	overview: string | null;
	season: number | null;
	episode: number | null;
	/** Path only (`/api/media/{id}/poster`), not yet usable as an `<img
	 * src>` on its own — this app's session token travels in a header,
	 * which an `<img>` tag can't attach, so every render site appends
	 * `?token=...` itself via `imageUrl` below rather than baking one
	 * fixed token into a value cached in `feed`. */
	poster_url: string;
	backdrop_url: string;
}

export interface HomeFeed {
	hero: MediaItem | null;
	recently_added: MediaItem[];
	top10: MediaItem[];
	recommended: MediaItem[];
	popular: MediaItem[];
}

export async function fetchHomeFeed(token: string): Promise<HomeFeed> {
	const response = await fetch('/api/home', { headers: buildAuthHeader(token) });
	if (!response.ok) {
		throw new Error(`GET /api/home failed with status ${response.status}`);
	}
	return (await response.json()) as HomeFeed;
}

/** Turns one of `MediaItem`'s bare route paths into something an `<img
 * src>` can actually load — see the `poster_url` doc comment above for why
 * the token has to be appended here rather than baked into the feed. Safe
 * to put in a URL query string as-is: session tokens are URL-safe-base64
 * already (see the backend's `token_from_query` doc comment), nothing to
 * percent-encode, but `encodeURIComponent` costs nothing and keeps this
 * correct even if that ever changes. */
export function imageUrl(path: string, token: string): string {
	return `${path}?token=${encodeURIComponent(token)}`;
}
