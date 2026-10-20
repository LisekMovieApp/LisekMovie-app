<!--
  "Next episode" card in the corner of the player during the end credits
  (and at the end). The primary button fills up while the countdown runs;
  at zero the player starts the next episode. "Watch credits" dismisses
  the card for this episode. The card is player UI (`data-player-ui`):
  the player's own shortcuts leave keys pressed on it alone.
-->
<script lang="ts">
	import { fly } from 'svelte/transition';
	import { t } from '$lib/i18n';
	import Icon from '$lib/components/Icon.svelte';
	import { imageUrl } from '$lib/media/api';
	import type { NextEpisode } from '$lib/playback/api';
	import { COUNTDOWN_SECONDS } from './nextUp';

	interface Props {
		next: NextEpisode;
		token: string;
		/** Seconds until the next episode starts by itself; null = waits for a press. */
		countdown: number | null;
		/** Focus the play button when the card appears (remotes). */
		autofocus?: boolean;
		onplay: () => void;
		ondismiss: () => void;
	}

	let { next, token, countdown, autofocus = false, onplay, ondismiss }: Props = $props();

	let imageFailed = $state(false);
	let playButton = $state<HTMLButtonElement>();

	$effect(() => {
		if (autofocus) playButton?.focus({ preventScroll: true });
	});
	const reduced =
		typeof window !== 'undefined' &&
		window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
	const label = $derived(
		next.end_episode
			? t('player.episode_range', {
					season: next.season,
					episode: next.episode,
					end: next.end_episode
				})
			: t('home.episode_short', { season: next.season, episode: next.episode })
	);
	const filled = $derived(
		countdown === null ? 0 : Math.min(1, Math.max(0, 1 - countdown / COUNTDOWN_SECONDS))
	);
</script>

<div
	class="absolute right-4 bottom-32 z-20 w-[min(26rem,calc(100vw-2rem))] overflow-hidden rounded-xl bg-neutral-950/90 text-white shadow-2xl ring-1 ring-white/15 backdrop-blur-xl md:right-8 md:bottom-36"
	role="region"
	aria-label={t('player.next_episode')}
	data-player-ui
	transition:fly={{ x: 40, duration: reduced ? 0 : 250 }}
>
	<div class="flex gap-4 p-4">
		<div class="relative aspect-video w-32 shrink-0 overflow-hidden rounded-md bg-surface md:w-40">
			{#if !imageFailed}
				<img
					src={imageUrl(next.backdrop_url, token)}
					alt=""
					class="size-full object-cover"
					onerror={() => (imageFailed = true)}
				/>
			{/if}
			<div class="absolute inset-0 grid place-items-center bg-black/30">
				<Icon name="play" class="size-8 drop-shadow" />
			</div>
		</div>
		<div class="min-w-0 flex-1">
			<p class="text-xs font-semibold tracking-[0.12em] text-gold uppercase">
				{t('player.next_episode')}
			</p>
			<p class="mt-1 truncate text-lg leading-tight font-bold">{label}</p>
			<p class="truncate text-sm text-neutral-300">{next.title}</p>
			{#if countdown !== null}
				<p class="mt-1 text-sm text-neutral-300 tabular-nums" aria-live="polite">
					{t('player.next_in', { seconds: Math.ceil(countdown) })}
				</p>
			{/if}
		</div>
	</div>
	<div class="flex gap-2 px-4 pb-4">
		<button
			type="button"
			class="relative flex flex-1 items-center justify-center gap-2 overflow-hidden rounded-md bg-white/90 px-4 py-2.5 font-semibold text-black hover:bg-white focus-visible:outline-gold"
			bind:this={playButton}
			onclick={onplay}
		>
			<span
				class="absolute inset-y-0 left-0 bg-gold transition-[width] duration-200 ease-linear"
				style:width={`${filled * 100}%`}
				aria-hidden="true"
			></span>
			<Icon name="play" class="relative size-5" />
			<span class="relative">{t('player.play_now')}</span>
		</button>
		<button
			type="button"
			class="rounded-md bg-white/10 px-4 py-2.5 font-semibold hover:bg-white/20"
			onclick={ondismiss}>{t('player.watch_credits')}</button
		>
	</div>
</div>
