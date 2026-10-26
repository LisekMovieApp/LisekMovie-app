<!--
  "Informacje" about one episode (Faza 10): its still, title, runtime,
  score, end time and synopsis, and the same Wideo / Dźwięk / Napisy picks
  as a film page — "Play" starts the episode right away with them. A
  spatial-navigation scope while open; Escape, Back or a click outside
  closes it.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { imageUrl } from '$lib/media/api';
	import {
		fetchItemDetails,
		formatRating,
		formatRuntime,
		preferredSubtitle,
		watchQuery,
		type ItemDetails
	} from '$lib/media/details';
	import { episodeEndsAt, episodeHeading, type EpisodeRow } from '$lib/media/series';
	import { formatClock } from '$lib/player/format';
	import { loadPreferences } from '$lib/player/preferences';
	import { navScope } from '$lib/spatial/spatial';
	import Icon from './Icon.svelte';
	import StreamPicker from './StreamPicker.svelte';

	interface Props {
		episode: EpisodeRow;
		/** "Sezon 1" or the series title, above the heading. */
		context: string;
		token: string;
		now: number;
		onclose: () => void;
	}

	let { episode, context, token, now, onclose }: Props = $props();

	let details = $state<ItemDetails | null>(null);
	let failed = $state(false);
	let audio = $state<number | null>(null);
	let subtitle = $state('');
	let stillFailed = $state(false);

	$effect(() => {
		const id = episode.id;
		let alive = true;
		details = null;
		failed = false;
		fetchItemDetails(token, id)
			.then((loaded) => {
				if (!alive) return;
				details = loaded;
				audio = loaded.default_audio_index ?? null;
				subtitle = preferredSubtitle(loaded.subtitle_tracks, loadPreferences().subtitleLanguage);
			})
			.catch(() => {
				if (alive) failed = true;
			});
		return () => {
			alive = false;
		};
	});

	$effect(() => {
		const onBack = (event: Event) => {
			event.preventDefault();
			onclose();
		};
		window.addEventListener('spatialback', onBack);
		return () => window.removeEventListener('spatialback', onBack);
	});

	const runtime = $derived(formatRuntime(episode.runtime_minutes));
	const rating = $derived(formatRating(episode.rating));
	const endTime = $derived.by(() => {
		const end = episodeEndsAt(now, episode);
		return end ? formatClock(end, getLocale()) : null;
	});
	const playHref = $derived(
		resolve('/watch/[id]', { id: episode.id }) + (details ? watchQuery({ audio, subtitle }) : '')
	);
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
	onclick={(e) => e.target === e.currentTarget && onclose()}
>
	<div
		role="dialog"
		aria-modal="true"
		aria-labelledby="episode-info-title"
		class="flex max-h-[92vh] w-full max-w-3xl flex-col overflow-y-auto rounded-lg border border-white/10 bg-surface text-neutral-100"
		use:navScope={{ autofocus: true }}
	>
		<div class="relative aspect-video w-full shrink-0 overflow-hidden bg-black">
			{#if !stillFailed}
				<img
					src={imageUrl(episode.thumb_url, token)}
					alt=""
					onerror={() => (stillFailed = true)}
					class="size-full object-cover"
				/>
			{/if}
			<div
				class="absolute inset-0 bg-gradient-to-t from-surface via-transparent to-transparent"
			></div>
			<button
				type="button"
				onclick={onclose}
				aria-label={t('player.close')}
				class="absolute top-3 right-3 grid size-10 place-items-center rounded-full bg-black/60 text-white hover:bg-black/80 focus-visible:outline-2 focus-visible:outline-gold"
			>
				<Icon name="close" class="size-5" />
			</button>
		</div>
		<div class="flex flex-col gap-4 p-5 md:p-6">
			<div>
				<p class="text-sm text-neutral-400">{context}</p>
				<h2 id="episode-info-title" class="text-xl font-semibold md:text-2xl">
					{episodeHeading(episode, t)}
				</h2>
				<div class="mt-2 flex flex-wrap items-center gap-2 text-sm text-neutral-200">
					{#if runtime}<span class="chip">{runtime}</span>{/if}
					{#if rating}
						<span class="chip flex items-center gap-1">
							<Icon name="star" class="size-4 text-gold" />
							{rating}
						</span>
					{/if}
					{#if endTime}<span class="chip">{t('details.ends_at', { time: endTime })}</span>{/if}
					{#if episode.watched}
						<span class="flex items-center gap-1 text-gold">
							<Icon name="check" class="size-4" />
							{t('series.watched')}
						</span>
					{/if}
				</div>
			</div>

			{#if details}
				<StreamPicker {details} bind:audio bind:subtitle idPrefix="episode-pick" />
			{:else if failed}
				<p class="text-sm text-neutral-400">{t('series.streams_unavailable')}</p>
			{/if}

			{#if episode.overview}
				<p class="text-sm leading-relaxed text-neutral-300 md:text-[0.95rem] md:leading-7">
					{episode.overview}
				</p>
			{/if}

			<div>
				<!-- eslint-disable svelte/no-navigation-without-resolve -- playHref builds on resolve() -->
				<a
					href={playHref}
					data-nav-default
					class="inline-flex items-center gap-2 rounded-md bg-gold px-6 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gold"
				>
					<Icon name="play" class="size-5" />
					{t('home.play')}
				</a>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			</div>
		</div>
	</div>
</div>

<style>
	.chip {
		border-radius: 0.3rem;
		background: rgb(255 255 255 / 0.1);
		padding: 0.15rem 0.6rem;
	}
</style>
