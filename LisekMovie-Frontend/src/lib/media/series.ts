// Series pages (Faza 10): season cards (`/api/media/{id}/season-cards`),
// a season's episodes with the viewer's progress
// (`/api/media/{id}/seasons/{n}/episodes`), whole-season flags and the
// helpers the season and episode views share.

import type { components } from '$lib/api/schema';
import { api } from '$lib/playback/api';
import { endsAt } from '$lib/player/format';

export type SeasonCard = components['schemas']['SeasonCardResponse'];
export type SeasonPage = components['schemas']['SeasonPageResponse'];
export type EpisodeRow = components['schemas']['EpisodeRowResponse'];

export async function fetchSeasonCards(token: string, itemId: string): Promise<SeasonCard[]> {
	const page = await api<{ items: SeasonCard[] }>(
		token,
		`/api/media/${encodeURIComponent(itemId)}/season-cards`
	);
	return page.items;
}

export function fetchSeasonPage(
	token: string,
	itemId: string,
	season: number
): Promise<SeasonPage> {
	return api<SeasonPage>(
		token,
		`/api/media/${encodeURIComponent(itemId)}/seasons/${season}/episodes`
	);
}

/** Watched and/or favourite for every episode of a season. */
export function setSeasonState(
	token: string,
	itemId: string,
	season: number,
	flags: { watched?: boolean; favorite?: boolean }
): Promise<void> {
	return api<void>(
		token,
		`/api/media/${encodeURIComponent(itemId)}/seasons/${season}/state`,
		'PUT',
		flags
	);
}

/** Watched and/or favourite for one item. */
export function setItemState(
	token: string,
	itemId: string,
	flags: { watched?: boolean; favorite?: boolean }
): Promise<void> {
	return api<void>(token, `/api/media/${encodeURIComponent(itemId)}/state`, 'PUT', flags);
}

/** The season's name from the provider, else "Sezon N" / "Odcinki specjalne". */
export function seasonTitle(
	season: { season: number; name?: string | null },
	t: (key: string, params?: Record<string, string | number>) => string
): string {
	if (season.name) return season.name;
	return season.season === 0
		? t('series.specials')
		: t('detail.season_label', { number: season.season });
}

/** "1. Najwyższy czas", "1-2. Pilot / Drugi", "3. Odcinek 3". */
export function episodeHeading(
	episode: Pick<EpisodeRow, 'episode' | 'end_episode' | 'title'>,
	t: (key: string, params?: Record<string, string | number>) => string
): string {
	const number =
		episode.end_episode !== null &&
		episode.end_episode !== undefined &&
		episode.end_episode > episode.episode
			? `${episode.episode}-${episode.end_episode}`
			: String(episode.episode);
	const title = episode.title ?? t('detail.episode_label', { number: episode.episode });
	return `${number}. ${title}`;
}

/** Watched part of an episode, 0..1, or `null` when not started (or done). */
export function progressOf(
	episode: Pick<EpisodeRow, 'position_seconds' | 'duration_seconds' | 'watched'>
): number | null {
	if (episode.watched || !episode.duration_seconds || episode.position_seconds <= 5) return null;
	return Math.min(1, episode.position_seconds / episode.duration_seconds);
}

/** When the episode ends if it starts (or resumes) now. */
export function episodeEndsAt(
	nowMs: number,
	episode: Pick<EpisodeRow, 'runtime_minutes' | 'position_seconds' | 'watched'>
): Date | null {
	if (!episode.runtime_minutes) return null;
	const resumeAt = !episode.watched && episode.position_seconds > 5 ? episode.position_seconds : 0;
	return endsAt(nowMs, Math.max(0, episode.runtime_minutes * 60 - resumeAt));
}
