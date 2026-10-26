<!--
  A season's episodes as a vertical list (Faza 10, Jellyfin's `listItem`):
  a large 16:9 thumbnail with a play button (and a progress bar for a
  started episode), "1. TITLE", runtime · ★ score · "Koniec o HH:MM",
  a short synopsis, and per-episode actions: Informacje (streams, then
  play), watched, favourite, a menu (download). Each row is a horizontal
  spatial-navigation group: left/right move along its controls, up/down
  between episodes.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { imageUrl } from '$lib/media/api';
	import { formatRating, formatRuntime } from '$lib/media/details';
	import {
		episodeEndsAt,
		episodeHeading,
		progressOf,
		setItemState,
		type EpisodeRow
	} from '$lib/media/series';
	import { formatClock } from '$lib/player/format';
	import { navGroup } from '$lib/spatial/spatial';
	import Icon from './Icon.svelte';
	import PlaystateButton from './PlaystateButton.svelte';
	import { TOGGLE_SHAPE, toggleTone } from './toggleTone';
	import MoreMenu from './MoreMenu.svelte';

	interface Props {
		episodes: EpisodeRow[];
		token: string;
		now: number;
		canDownload: boolean;
		oninfo: (episode: EpisodeRow) => void;
		ondownload: (episode: EpisodeRow) => void;
	}

	let { episodes = $bindable(), token, now, canDownload, oninfo, ondownload }: Props = $props();

	let busy = $state<string | null>(null);
	let failed = $state(false);
	let thumbFailed = $state<Record<string, boolean>>({});

	async function toggle(episode: EpisodeRow, key: 'watched' | 'favorite') {
		if (busy) return;
		busy = episode.id;
		failed = false;
		const value = !episode[key];
		try {
			await setItemState(token, episode.id, { [key]: value });
			episodes = episodes.map((e) =>
				e.id !== episode.id
					? e
					: key === 'watched'
						? { ...e, watched: value, position_seconds: 0 }
						: { ...e, favorite: value }
			);
		} catch {
			failed = true;
		} finally {
			busy = null;
		}
	}

	const iconShape = `${TOGGLE_SHAPE} size-10`;
	const icon = `${iconShape} ${toggleTone('watched', false, 'subtle')}`;
</script>

<ol class="flex flex-col gap-2">
	{#each episodes as episode (episode.id)}
		{@const runtime = formatRuntime(episode.runtime_minutes)}
		{@const rating = formatRating(episode.rating)}
		{@const end = episodeEndsAt(now, episode)}
		{@const progress = progressOf(episode)}
		{@const heading = episodeHeading(episode, t)}
		<li
			class="group flex flex-col gap-3 rounded-lg p-2 transition-colors focus-within:bg-white/5 hover:bg-white/5 sm:flex-row sm:items-start sm:gap-5 md:p-3"
			use:navGroup={{ contain: 'horizontal' }}
		>
			<a
				href={resolve('/watch/[id]', { id: episode.id })}
				class="relative block aspect-video w-full shrink-0 overflow-hidden rounded-md bg-black ring-gold outline-none focus-visible:ring-[3px] sm:w-56 md:w-72 lg:w-80"
				aria-label={t('series.play_episode', { name: heading })}
			>
				{#if !thumbFailed[episode.id]}
					<img
						src={imageUrl(episode.thumb_url, token)}
						alt=""
						loading="lazy"
						onerror={() => (thumbFailed = { ...thumbFailed, [episode.id]: true })}
						class="size-full object-cover transition-transform duration-300 group-hover:scale-105 {episode.watched
							? 'opacity-60'
							: ''}"
					/>
				{/if}
				<span
					class="absolute inset-0 grid place-items-center bg-black/30 text-white opacity-80 transition-opacity group-hover:opacity-100"
					aria-hidden="true"
				>
					<span
						class="grid size-14 place-items-center rounded-full bg-black/55 ring-2 ring-white/70 backdrop-blur-sm"
					>
						<Icon name="play" class="ml-0.5 size-7" />
					</span>
				</span>
				{#if episode.watched}
					<span
						class="absolute top-2 right-2 grid size-7 place-items-center rounded-full bg-gold text-black"
						title={t('series.watched')}
					>
						<Icon name="check" class="size-4" />
					</span>
				{/if}
				{#if progress !== null}
					<span class="absolute inset-x-0 bottom-0 h-1 bg-white/25" aria-hidden="true">
						<span class="block h-full bg-gold" style:width={`${Math.round(progress * 100)}%`}
						></span>
					</span>
				{/if}
			</a>

			<div class="min-w-0 flex-1">
				<div class="flex items-start justify-between gap-3">
					<div class="min-w-0">
						<h3 class="font-semibold text-base text-neutral-50 uppercase md:text-lg">
							{heading}
						</h3>
						<div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-neutral-300">
							{#if runtime}<span>{runtime}</span>{/if}
							{#if rating}
								<span class="flex items-center gap-1">
									<Icon name="star" class="size-4 text-gold" />
									{rating}
								</span>
							{/if}
							{#if end}
								<span>{t('details.ends_at', { time: formatClock(end, getLocale()) })}</span>
							{/if}
						</div>
					</div>
					<div class="hidden shrink-0 items-center sm:flex">
						<button
							type="button"
							class={icon}
							aria-label={t('series.info')}
							title={t('series.info')}
							onclick={() => oninfo(episode)}
						>
							<Icon name="info" class="size-5" />
						</button>
						<PlaystateButton
							played={episode.watched}
							size="md"
							surface="subtle"
							disabled={busy !== null}
							onclick={() => toggle(episode, 'watched')}
						/>
						<button
							type="button"
							class="{iconShape} {toggleTone('favorite', episode.favorite, 'subtle')}"
							disabled={busy !== null}
							aria-pressed={episode.favorite}
							aria-label={t(episode.favorite ? 'player.unfavorite' : 'player.favorite')}
							title={t(episode.favorite ? 'player.unfavorite' : 'player.favorite')}
							onclick={() => toggle(episode, 'favorite')}
						>
							<Icon name={episode.favorite ? 'heart_filled' : 'heart'} class="size-5" />
						</button>
						<MoreMenu
							entries={[
								{ labelKey: 'series.info', onselect: () => oninfo(episode) },
								...(canDownload
									? [{ labelKey: 'download.button', onselect: () => ondownload(episode) }]
									: [])
							]}
						/>
					</div>
				</div>
				{#if episode.overview}
					<p class="mt-2 line-clamp-3 text-sm leading-relaxed text-neutral-400 md:text-[0.95rem]">
						{episode.overview}
					</p>
				{/if}
				<div class="mt-2 flex items-center gap-1 sm:hidden">
					<button
						type="button"
						class={icon}
						aria-label={t('series.info')}
						onclick={() => oninfo(episode)}
					>
						<Icon name="info" class="size-5" />
					</button>
					<PlaystateButton
						played={episode.watched}
						size="md"
						surface="subtle"
						tooltip={false}
						disabled={busy !== null}
						onclick={() => toggle(episode, 'watched')}
					/>
					<button
						type="button"
						class="{iconShape} {toggleTone('favorite', episode.favorite, 'subtle')}"
						disabled={busy !== null}
						aria-pressed={episode.favorite}
						aria-label={t(episode.favorite ? 'player.unfavorite' : 'player.favorite')}
						onclick={() => toggle(episode, 'favorite')}
					>
						<Icon name={episode.favorite ? 'heart_filled' : 'heart'} class="size-5" />
					</button>
					{#if canDownload}
						<MoreMenu
							entries={[{ labelKey: 'download.button', onselect: () => ondownload(episode) }]}
						/>
					{/if}
				</div>
			</div>
		</li>
	{/each}
</ol>
{#if failed}
	<p role="alert" class="mt-3 text-sm text-gold">{t('player.save_failed')}</p>
{/if}
