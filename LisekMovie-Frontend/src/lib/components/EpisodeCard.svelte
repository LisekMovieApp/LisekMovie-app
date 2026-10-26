<!--
  A landscape (16:9) card for an episode (Faza 11, search results): its
  still or a frame, the series title and "S2:E5 - Tytuł" underneath,
  linking to the episode's season page.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { imageUrl } from '$lib/media/api';
	import { episodeLabel, type EpisodeResult } from '$lib/media/search';

	interface Props {
		episode: EpisodeResult;
		token: string;
	}

	let { episode, token }: Props = $props();
	let failed = $state(false);
</script>

<a
	href={resolve('/media/[id]/season/[season]', {
		id: episode.id,
		season: String(episode.season)
	})}
	class="group block w-[220px] shrink-0 outline-none sm:w-[260px] md:w-[300px]"
>
	<div
		class="relative aspect-video overflow-hidden rounded-md bg-surface ring-gold transition-[transform,box-shadow] duration-200 group-hover:shadow-[0_10px_30px_rgb(0_0_0/0.6)] group-focus-visible:scale-[1.05] group-focus-visible:ring-[3px]"
	>
		{#if !failed}
			<img
				src={imageUrl(episode.thumb_url, token)}
				alt=""
				loading="lazy"
				onerror={() => (failed = true)}
				class="size-full object-cover transition-transform duration-300 group-hover:scale-105"
			/>
		{/if}
	</div>
	<p class="mt-2 truncate text-xs text-neutral-400">{episode.series_title}</p>
	<p class="truncate text-sm font-medium text-neutral-100 group-focus-visible:text-gold">
		{episodeLabel(episode)}
	</p>
</a>
