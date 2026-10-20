<!--
  "Continue watching": everything started and not finished, newest first
  as `/api/me/list` returns it. Cards go straight to the player, which
  resumes where the telemetry last saved the position, and show how much
  is left. Renders nothing when there is nothing to continue.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { entryAsItem, inProgress, minutesLeft, progressOf } from '$lib/media/list';
	import type { ListEntry } from '$lib/playback/api';
	import Carousel from './Carousel.svelte';
	import MediaCard from './MediaCard.svelte';

	interface Props {
		entries: ListEntry[];
		token: string;
	}

	let { entries, token }: Props = $props();

	const items = $derived(entries.filter(inProgress));

	function caption(entry: ListEntry): string {
		const left = t('home.minutes_left', { minutes: minutesLeft(entry) });
		return entry.season !== null && entry.episode !== null
			? `${t('home.episode_short', { season: entry.season, episode: entry.episode })} · ${left}`
			: left;
	}
</script>

{#if items.length > 0}
	<section class="group/row relative">
		<h2 class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8 md:text-xl">
			{t('home.row_continue')}
		</h2>
		<Carousel>
			{#each items as entry (entry.id)}
				<MediaCard
					item={entryAsItem(entry)}
					{token}
					href={resolve('/watch/[id]', { id: entry.id })}
					caption={caption(entry)}
					progress={progressOf(entry)}
				/>
			{/each}
		</Carousel>
	</section>
{/if}
