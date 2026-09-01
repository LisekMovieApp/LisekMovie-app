// Client-side calls for the home page's feed (`GET /api/home`) and a single
// item's own public detail (`GET /api/media/{id}`) — the latter added so a
// direct-linked or older item (not necessarily among the handful of rows
// the home feed returns) can still be fetched on its own for the Detail
// View. See `lisek-server/src/media.rs` for the exact response shapes both
// mirror — both endpoints return the same `MediaItemResponse` shape.

import { buildAuthHeader } from '$lib/auth';

export interface MediaItem {
	id: string;
	/**
	 * Runda 8: `'episode'` no longer appears here — the backend never hands
	 * one back on its own any more (see `lisek_db`'s `home_candidates` CTE
	 * doc comment). `'series'` is new: a whole TV show, represented by one
	 * of its episodes under the hood, but never distinguishable from that
	 * on the frontend's side — `year`/`season`/`episode` are always `null`
	 * for it (a show has no single year or episode number of its own), and
	 * `id` is what `fetchSeriesSeasons` below takes to load its "Sezony".
	 */
	item_type: 'movie' | 'series';
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
	/** Same "path only" contract as `poster_url`/`backdrop_url` above.
	 * Present for every item, but many won't have a real logo image behind
	 * it — the Detail View and `HeroBanner` both handle a failed load by
	 * falling back to the plain text title, the same `<img onerror>` idiom
	 * `HeroBanner` already uses for a missing backdrop. */
	logo_url: string;
	/** Set only for a `'series'` card on the home feed — see
	 * `lisek_db::classify_series_badge`'s doc comment for exactly what each
	 * value means and the (documented, arbitrary) 7-day window it's based
	 * on. `null` for a movie always, and for a series with nothing recently
	 * added, and for anything the Detail View's own `fetchMediaDetail`
	 * returns (the backend only computes it for home-feed rows — see
	 * `MediaItemResponse::badge`'s own doc comment). */
	badge: 'new_series' | 'new_season' | 'new_episodes' | null;
}

/** One season of a `'series'` item, as `fetchSeriesSeasons` returns it —
 * mirrors `lisek-server`'s `SeriesSeasonResponse`/`SeriesEpisodeResponse`.
 * Deliberately thin: see `lisek_db::list_series_seasons`'s doc comment for
 * exactly why an episode here carries no title/thumbnail/runtime/overview
 * of its own — none of that exists anywhere in this app yet, and this type
 * only carries what's real. */
export interface SeriesSeason {
	season: number;
	episodes: SeriesEpisode[];
}

export interface SeriesEpisode {
	id: string;
	episode: number;
	/** Set only for a multi-episode file recognized as a range at scan
	 * time (e.g. a file matching `S01E01-E02`). */
	end_episode: number | null;
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

/** Thrown by `fetchMediaDetail` specifically for a 404 — so the Detail View
 * can show "this title doesn't exist" rather than the generic "couldn't
 * load, try again" it shows for every other failure (a real network error
 * is worth retrying; a deleted/never-existed item never will be). */
export class MediaNotFoundError extends Error {}

export async function fetchMediaDetail(token: string, itemId: string): Promise<MediaItem> {
	const response = await fetch(`/api/media/${encodeURIComponent(itemId)}`, {
		headers: buildAuthHeader(token)
	});
	if (response.status === 404) {
		throw new MediaNotFoundError();
	}
	if (!response.ok) {
		throw new Error(`GET /api/media/${itemId} failed with status ${response.status}`);
	}
	return (await response.json()) as MediaItem;
}

/** The "Sezony" section's data for one series `itemId` — an empty array
 * for a movie, or an id the backend doesn't recognize (see
 * `lisek_db::list_series_seasons`'s doc comment on why that's a plain
 * empty result rather than a distinct error here too). Only ever called
 * for an item whose `item_type` is already known to be `'series'` — see
 * the Detail View's own `onMount`/`load`. */
export async function fetchSeriesSeasons(token: string, itemId: string): Promise<SeriesSeason[]> {
	const response = await fetch(`/api/media/${encodeURIComponent(itemId)}/seasons`, {
		headers: buildAuthHeader(token)
	});
	if (!response.ok) {
		throw new Error(`GET /api/media/${itemId}/seasons failed with status ${response.status}`);
	}
	return (await response.json()) as SeriesSeason[];
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
