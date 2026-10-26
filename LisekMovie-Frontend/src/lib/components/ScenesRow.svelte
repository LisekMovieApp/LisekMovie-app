<!--
  "Sceny" on a detail page (Faza 9): the file's chapters as a horizontal
  carousel — a frame from each scene, its name and where it starts.
  Choosing one opens the player right there (with the streams picked on
  the page). Files without chapters get evenly spaced scenes from the
  server, named "Scena N". Built on `Carousel`, so the row is a
  spatial-navigation group for TV remotes. Thumbnails are cut on the
  server the first time they are asked for; until one arrives the card
  shows its time on a plain tile.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { imageUrl } from '$lib/media/api';
	import { formatTimestamp, type Chapter } from '$lib/media/details';
	import Carousel from './Carousel.svelte';
	import Icon from './Icon.svelte';

	interface Props {
		chapters: Chapter[];
		token: string;
		/** The `/watch/[id]?...` link for a scene starting at `seconds`. */
		hrefFor: (seconds: number) => string;
	}

	let { chapters, token, hrefFor }: Props = $props();

	let failed = $state<Record<number, boolean>>({});
</script>

{#if chapters.length > 0}
	<section class="group/row relative mt-2" aria-labelledby="scenes-title">
		<h2 id="scenes-title" class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">
			{t('details.scenes')}
		</h2>
		<Carousel>
			<!-- eslint-disable svelte/no-navigation-without-resolve -- hrefFor() builds on resolve() -->
			{#each chapters as chapter (chapter.index)}
				<a
					href={hrefFor(chapter.start_seconds)}
					class="group block w-[200px] shrink-0 outline-none sm:w-[240px] md:w-[280px]"
					aria-label={t('details.play_scene', {
						name: chapter.title ?? t('details.scene_number', { number: chapter.index + 1 }),
						time: formatTimestamp(chapter.start_seconds)
					})}
				>
					<div
						class="relative aspect-video overflow-hidden rounded-md bg-surface ring-gold transition-[transform,box-shadow] duration-200 group-hover:shadow-[0_10px_30px_rgb(0_0_0/0.6)] group-focus-visible:scale-[1.06] group-focus-visible:ring-[3px]"
					>
						{#if !failed[chapter.index]}
							<img
								src={imageUrl(chapter.image_url, token)}
								alt=""
								loading="lazy"
								onerror={() => (failed = { ...failed, [chapter.index]: true })}
								class="size-full object-cover transition-transform duration-300 group-hover:scale-105"
							/>
						{/if}
						<span
							class="absolute inset-0 grid place-items-center bg-black/40 text-white opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
							aria-hidden="true"
						>
							<Icon name="play" class="size-10 drop-shadow" />
						</span>
						<span
							class="absolute right-1.5 bottom-1.5 rounded bg-black/75 px-1.5 py-0.5 text-xs text-neutral-100 tabular-nums"
							>{formatTimestamp(chapter.start_seconds)}</span
						>
					</div>
					<p class="mt-2 truncate text-sm text-neutral-200 group-focus-visible:text-gold">
						{chapter.title ?? t('details.scene_number', { number: chapter.index + 1 })}
					</p>
				</a>
			{/each}
			<!-- eslint-enable svelte/no-navigation-without-resolve -->
		</Carousel>
	</section>
{/if}
