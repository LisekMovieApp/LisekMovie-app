<!--
  "Sezony" of a series page (Faza 10): a responsive grid of season cards —
  the season poster, the number of episodes still to watch in the corner
  (a check when all are watched), the season's name underneath. Hovering a
  card, or reaching it with a TV remote (focus inside it), shows its
  actions: Play (the first unwatched episode, from its resume point),
  watched and favourite for the whole season, and a menu. The grid is one
  spatial-navigation group that remembers the card left last.
-->
<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { imageUrl } from '$lib/media/api';
	import { seasonTitle, setSeasonState, type SeasonCard } from '$lib/media/series';
	import { navGroup } from '$lib/spatial/spatial';
	import Icon from './Icon.svelte';
	import PlaystateButton from './PlaystateButton.svelte';
	import { PLAY_TONE, TOGGLE_SHAPE, toggleTone } from './toggleTone';
	import MoreMenu from './MoreMenu.svelte';

	interface Props {
		seriesId: string;
		seasons: SeasonCard[];
		token: string;
		/** After a whole season changed (watched): the page reloads its state. */
		onchange?: () => void;
	}

	let { seriesId, seasons = $bindable(), token, onchange }: Props = $props();

	let busy = $state<number | null>(null);
	let failed = $state(false);
	let posterFailed = $state<Record<number, boolean>>({});

	async function toggle(card: SeasonCard, key: 'watched' | 'favorite') {
		if (busy !== null) return;
		busy = card.season;
		failed = false;
		const value = key === 'watched' ? card.unwatched_count > 0 : !card.favorite;
		try {
			await setSeasonState(token, seriesId, card.season, { [key]: value });
			seasons = seasons.map((s) =>
				s.season !== card.season
					? s
					: key === 'watched'
						? { ...s, unwatched_count: value ? 0 : s.episode_count }
						: { ...s, favorite: value }
			);
			if (key === 'watched') onchange?.();
		} catch {
			failed = true;
		} finally {
			busy = null;
		}
	}

	/** A remote reaches a card's actions with ↓ from the poster and goes
	 * back with ↑: the buttons sit inside the poster's rectangle, where
	 * the spatial navigation's geometry cannot pick them. */
	function intoActions(event: KeyboardEvent) {
		if (event.key !== 'ArrowDown') return;
		const card = (event.currentTarget as HTMLElement).closest('[data-season-card]');
		const first = card?.querySelector<HTMLElement>('.actions a, .actions button');
		if (!first) return;
		event.preventDefault();
		first.focus({ preventScroll: true });
	}

	function backToPoster(event: KeyboardEvent) {
		// Inside the open "⋮" menu ↑ moves between its entries.
		if (event.key !== 'ArrowUp' || (event.target as HTMLElement).closest('[role=menu]')) return;
		const link = (event.currentTarget as HTMLElement)
			.closest('[data-season-card]')
			?.querySelector<HTMLElement>('a[data-poster]');
		if (!link) return;
		event.preventDefault();
		link.focus({ preventScroll: true });
	}

	const seasonHref = (season: number) =>
		resolve('/media/[id]/season/[season]', { id: seriesId, season: String(season) });

	const action = `${TOGGLE_SHAPE} size-9`;
</script>

{#if seasons.length > 0}
	<section aria-labelledby="seasons-title">
		<h2 id="seasons-title" class="mb-4 text-lg font-semibold text-neutral-100 md:text-xl">
			{t('detail.seasons_title')}
		</h2>
		<div
			class="grid grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))] gap-x-4 gap-y-6 sm:grid-cols-[repeat(auto-fill,minmax(10rem,1fr))] md:grid-cols-[repeat(auto-fill,minmax(11.5rem,1fr))]"
			use:navGroup={{}}
		>
			{#each seasons as card (card.season)}
				{@const title = seasonTitle(card, t)}
				<div class="group relative min-w-0" data-season-card>
					<a
						href={seasonHref(card.season)}
						data-poster
						onkeydown={intoActions}
						class="relative block aspect-[2/3] overflow-hidden rounded-md bg-surface ring-gold transition-[transform,box-shadow] duration-200 outline-none group-hover:shadow-[0_10px_30px_rgb(0_0_0/0.6)] focus-visible:ring-[3px]"
						aria-label={t('series.season_card', {
							name: title,
							count: card.episode_count
						})}
					>
						{#if !posterFailed[card.season]}
							<img
								src={imageUrl(card.poster_url, token)}
								alt=""
								loading="lazy"
								onerror={() => (posterFailed = { ...posterFailed, [card.season]: true })}
								class="size-full object-cover transition-transform duration-300 group-hover:scale-105"
							/>
						{:else}
							<span
								class="absolute inset-0 grid place-items-center px-2 text-center text-neutral-400"
								>{title}</span
							>
						{/if}
						<span
							class="absolute top-1.5 right-1.5 grid h-7 min-w-7 place-items-center rounded-full px-1.5 text-xs font-semibold {card.unwatched_count >
							0
								? 'bg-gold text-black'
								: 'bg-black/70 text-gold'}"
							title={card.unwatched_count > 0
								? t('series.unwatched_count', { count: card.unwatched_count })
								: t('series.all_watched')}
						>
							{#if card.unwatched_count > 0}{card.unwatched_count}{:else}<Icon
									name="check"
									class="size-4"
								/>{/if}
						</span>
						<span
							class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/80 via-black/10 to-transparent opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100"
							aria-hidden="true"
						></span>
					</a>
					<div
						role="toolbar"
						tabindex="-1"
						aria-label={title}
						onkeydown={backToPoster}
						class="actions pointer-events-none absolute inset-x-0 top-0 flex aspect-[2/3] items-end justify-center gap-1.5 pb-3 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100"
					>
						<a
							href={resolve('/watch/[id]', { id: card.play_id })}
							class="{action} {PLAY_TONE}"
							aria-label={t('series.play_season', { name: title })}
							title={t('home.play')}
						>
							<Icon name="play" class="size-5" />
						</a>
						<PlaystateButton
							played={card.unwatched_count === 0}
							size="sm"
							surface="overlay"
							disabled={busy !== null}
							onclick={() => toggle(card, 'watched')}
						/>
						<button
							type="button"
							class="{action} {toggleTone('favorite', card.favorite, 'overlay')}"
							disabled={busy !== null}
							aria-pressed={card.favorite}
							aria-label={t(card.favorite ? 'player.unfavorite' : 'player.favorite')}
							title={t(card.favorite ? 'player.unfavorite' : 'player.favorite')}
							onclick={() => toggle(card, 'favorite')}
						>
							<Icon name={card.favorite ? 'heart_filled' : 'heart'} class="size-5" />
						</button>
						<MoreMenu
							compact
							entries={[
								{
									labelKey: 'series.open_season',
									onselect: () => void goto(seasonHref(card.season))
								},
								{
									labelKey:
										card.unwatched_count === 0 ? 'series.mark_unwatched' : 'series.mark_watched',
									onselect: () => toggle(card, 'watched')
								}
							]}
						/>
					</div>
					<p
						class="mt-2 truncate text-sm font-medium text-neutral-100 group-focus-within:text-gold"
					>
						{title}
					</p>
					{#if card.air_year}
						<p class="text-xs text-neutral-500">{card.air_year}</p>
					{/if}
				</div>
			{/each}
		</div>
		{#if failed}
			<p role="alert" class="mt-3 text-sm text-gold">{t('player.save_failed')}</p>
		{/if}
	</section>
{/if}

<style>
	/* The actions sit over the poster without blocking it: only the buttons
	   take clicks, and only with a mouse — a tap on a phone opens the
	   season. A remote reaches them by focus either way. */
	@media (pointer: fine) {
		.actions > :global(*) {
			pointer-events: auto;
		}
	}
</style>
