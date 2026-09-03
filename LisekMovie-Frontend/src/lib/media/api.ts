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
	/** Set for a `'series'` card (see `lisek_db::classify_series_badge`'s doc
	 * comment for what each `new_*` value means and the documented,
	 * arbitrary 7-day window it's based on), or for a `'movie'` card added
	 * within that same window (`'new_movie'`, Runda 9 — see
	 * `lisek-server::media::is_recently_added`). `null` for anything the
	 * Detail View's own `fetchMediaDetail` returns — the backend only
	 * computes a badge for browse-card rows (the home feed, and as of
	 * Runda 9 `fetchLibraryItems`'s grid too — see
	 * `MediaItemResponse::badge`'s own doc comment). */
	badge: 'new_series' | 'new_season' | 'new_episodes' | 'new_movie' | null;
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

/** One library as the Navbar/`/library/[id]` grid see it — deliberately
 * thinner than `$lib/admin/libraries`' own `Library`: no `root_paths` or
 * `save_metadata_in_media_folder`, since those are admin-only concerns the
 * public `GET /api/libraries` never sends to begin with (Runda 9 — see
 * `lisek-server::media::PublicLibraryResponse`'s doc comment). */
export interface LibrarySummary {
	id: string;
	name: string;
	library_type: 'movies' | 'tv_shows';
}

/** The Navbar's own data source (Runda 9): every configured library, in the
 * admin's shared display order — same `ORDER BY sort_order, name` the
 * Admin Panel's own library list already uses, so the two can never
 * disagree about which library comes first. Called once, client-side, from
 * `Navbar.svelte`'s own `onMount` — see that component's top comment for
 * why a failed fetch here just leaves the static nav items showing rather
 * than breaking the whole bar. */
export async function fetchLibraries(token: string): Promise<LibrarySummary[]> {
	const response = await fetch('/api/libraries', { headers: buildAuthHeader(token) });
	if (!response.ok) {
		throw new Error(`GET /api/libraries failed with status ${response.status}`);
	}
	return (await response.json()) as LibrarySummary[];
}

/** How `LibraryAllSection`'s "all items" grid can be ordered (Runda 10) —
 * mirrors `lisek_db::LibrarySort`'s three variants one-for-one; the raw
 * strings are exactly what `?sort=` expects (see
 * `lisek-server::media::parse_library_sort`). */
export type LibrarySort = 'title_asc' | 'title_desc' | 'recent';

export interface FetchLibraryItemsOptions {
	/** 1-based; omitted defaults to page 1 on the backend. */
	page?: number;
	sort?: LibrarySort;
	/** A single letter (any case — the backend uppercases it); omitted or
	 * invalid means "no filter". */
	letter?: string;
}

export interface LibraryItems {
	library: LibrarySummary;
	items: MediaItem[];
	/** Total matching items across every page (already grouped — a series
	 * counts once, not once per episode) — Runda 10, backs the "1–100 z
	 * 765" counter. */
	total: number;
	page: number;
	page_size: number;
}

/** Thrown by `fetchLibraryItems`/`fetchLibraryRecent`/`fetchLibraryRecommended`/
 * `fetchLibraryPopular` specifically for a 404 — the `/library/[id]` page's
 * own "this library doesn't exist" case, same `MediaNotFoundError`-style
 * split as `fetchMediaDetail`'s below: a real network error is worth a
 * retry button, a deleted/never-existed library id never will be. */
export class LibraryNotFoundError extends Error {}

/** One page of one library's whole browsable grid (Runda 9; paginated,
 * sorted and letter-filterable as of Runda 10) — movies and series (never
 * bare episodes, same `home_candidates`-style grouping the home feed
 * already uses). See `lisek-server::media::library_items`'s doc comment for
 * the response shape and why the library's own name/type rides along
 * instead of a second round trip. */
export async function fetchLibraryItems(
	token: string,
	libraryId: string,
	options: FetchLibraryItemsOptions = {}
): Promise<LibraryItems> {
	const params = new URLSearchParams();
	if (options.page !== undefined) params.set('page', String(options.page));
	if (options.sort !== undefined) params.set('sort', options.sort);
	if (options.letter !== undefined) params.set('letter', options.letter);
	const query = params.toString();
	const url = `/api/libraries/${encodeURIComponent(libraryId)}/items${query ? `?${query}` : ''}`;

	const response = await fetch(url, { headers: buildAuthHeader(token) });
	if (response.status === 404) {
		throw new LibraryNotFoundError();
	}
	if (!response.ok) {
		throw new Error(`GET ${url} failed with status ${response.status}`);
	}
	return (await response.json()) as LibraryItems;
}

/** One library's "Ostatnio dodane"/"Polecane dla Ciebie"/"Popularne teraz"
 * row (Runda 10) — the shape `fetchLibraryRecent`/`fetchLibraryRecommended`/
 * `fetchLibraryPopular` all return: no pagination fields, since these are
 * small curated rows, not the paginated "all items" grid. */
export interface LibraryRowItems {
	library: LibrarySummary;
	items: MediaItem[];
}

async function fetchLibraryRow(
	token: string,
	libraryId: string,
	segment: 'recent' | 'recommended' | 'popular'
): Promise<LibraryRowItems> {
	const url = `/api/libraries/${encodeURIComponent(libraryId)}/${segment}`;
	const response = await fetch(url, { headers: buildAuthHeader(token) });
	if (response.status === 404) {
		throw new LibraryNotFoundError();
	}
	if (!response.ok) {
		throw new Error(`GET ${url} failed with status ${response.status}`);
	}
	return (await response.json()) as LibraryRowItems;
}

/** `/library/[id]`'s "Ostatnio dodane" row — the home feed's own
 * `list_recently_added` logic, scoped to this library (Runda 10). */
export function fetchLibraryRecent(token: string, libraryId: string): Promise<LibraryRowItems> {
	return fetchLibraryRow(token, libraryId, 'recent');
}

/** `/library/[id]`'s "Polecane dla Ciebie" row — the home feed's own
 * `list_recommended` logic, scoped to this library (Runda 10); never mixes
 * in another library's items. */
export function fetchLibraryRecommended(
	token: string,
	libraryId: string
): Promise<LibraryRowItems> {
	return fetchLibraryRow(token, libraryId, 'recommended');
}

/** `/library/[id]`'s "Popularne teraz" row — the home feed's own
 * `list_popular` logic, scoped to this library (Runda 10). */
export function fetchLibraryPopular(token: string, libraryId: string): Promise<LibraryRowItems> {
	return fetchLibraryRow(token, libraryId, 'popular');
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
