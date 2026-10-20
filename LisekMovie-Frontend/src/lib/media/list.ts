/**
 * "My List" entries (`/api/me/list`) shown with the same cards as the
 * home feed: an entry carries only ids, titles and progress, so the image
 * paths are rebuilt the way the server builds them for feed items.
 */
import type { MediaItem } from './api';
import type { ListEntry } from '$lib/playback/api';

export function entryAsItem(entry: ListEntry): MediaItem {
	return {
		id: entry.id,
		item_type: 'movie',
		title: entry.title,
		year: null,
		overview: null,
		season: entry.season,
		episode: entry.episode,
		poster_url: `/api/media/${entry.id}/poster`,
		backdrop_url: `/api/media/${entry.id}/backdrop`,
		logo_url: `/api/media/${entry.id}/logo`,
		badge: null
	};
}

/** Started and not finished: what "continue watching" lists. */
export function inProgress(entry: ListEntry): boolean {
	return !entry.watched && entry.position_seconds > 0 && entry.duration_seconds > 0;
}

/** Watched fraction 0..1, or null without a known duration. */
export function progressOf(entry: ListEntry): number | null {
	return entry.duration_seconds > 0
		? Math.min(1, entry.position_seconds / entry.duration_seconds)
		: null;
}

/** Whole minutes left (at least 1). */
export function minutesLeft(entry: ListEntry): number {
	return Math.max(1, Math.round((entry.duration_seconds - entry.position_seconds) / 60));
}
