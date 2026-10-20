<!--
  The "Top 10" row — same sliding-carousel mechanics as MediaRow (via the
  shared `Carousel.svelte`), but each poster carries a giant rank number
  instead of a plain title, matching the reference layout. The rank numbers
  are real, final UI regardless of loading state (a position 1-10 is
  structural, not data waiting on the library scanner) — only the poster
  art underneath depends on `items`: skeleton while `null` (still loading),
  a real poster once loaded (see `lisek-db::list_top_today`'s doc comment
  for what "top 10" actually means without real popularity tracking — an
  honest "newest releases" ranking, not a fabricated trending chart).
  Posters here are portrait, not the landscape aspect-video of
  SkeletonCard/MediaRow: deliberate, matching the reference's
  Top-10-specific visual language rather than reusing the generic
  thumbnail shape — so this uses the poster image, not the backdrop.

  Numbers and posters are centered on each other (`items-center`, on both
  the row and each item's own wrapper) rather than aligned to a shared
  edge: a huge numeral's glyph doesn't fill its line box symmetrically (font
  metrics leave more space below the baseline than above the cap-height),
  so bottom- or top-aligning it against the poster's plain rectangle reads
  as visibly off — centering is what actually looks centered here.

  Each real entry's whole `<a>` (number + poster together, not just the
  poster) navigates to that item's Detail View (`/media/[id]`) — matching
  how the number is visually part of the same clickable unit as the art
  underneath it, per the reference layout.
-->
<script lang="ts">
	import { SvelteSet } from 'svelte/reactivity';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import Carousel from './Carousel.svelte';
	import { imageUrl, type MediaItem } from '$lib/media/api';

	interface Props {
		/** `null` = still loading (renders 10 skeletons); `[]` = loaded and empty (renders nothing). */
		items: MediaItem[] | null;
		token: string;
	}

	let { items, token }: Props = $props();

	const skeletonRanks = Array.from({ length: 10 }, (_, i) => i + 1);

	// One item can fail to load its poster independently of the others —
	// tracked by item id rather than array index so a mid-list failure
	// doesn't get mixed up with a different item after a feed reload.
	// `SvelteSet` is already reactive on its own — no `$state` wrapper.
	let failedIds = new SvelteSet<string>();
</script>

{#if items === null || items.length > 0}
	<section class="group/row relative">
		<h2 class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">{t('home.row_top10')}</h2>

		<Carousel>
			{#if items === null}
				{#each skeletonRanks as rank (rank)}
					<div class="flex shrink-0 items-center">
						<!--
						  Transparent fill + a gold stroke, rather than a plain
						  translucent grey number: the outline reads clearly against any
						  poster art behind it without competing for attention with the
						  poster itself once real thumbnails land here.
						-->
						<span
							aria-hidden="true"
							class="-mr-5 text-[6.5rem] leading-none font-black text-transparent select-none [-webkit-text-stroke:3px_var(--color-gold)] sm:-mr-7 sm:text-[8.5rem] md:-mr-9 md:text-[10rem]"
						>
							{rank}
						</span>
						<div
							class="aspect-[2/3] w-[127px] shrink-0 animate-pulse rounded-md bg-surface sm:w-[168px] md:w-[192px]"
						></div>
					</div>
				{/each}
			{:else}
				{#each items as item, i (item.id)}
					<a
						href={resolve('/media/[id]', { id: item.id })}
						class="group flex shrink-0 items-center outline-none"
					>
						<span
							aria-hidden="true"
							class="-mr-5 text-[6.5rem] leading-none font-black text-transparent select-none [-webkit-text-stroke:3px_var(--color-gold)] sm:-mr-7 sm:text-[8.5rem] md:-mr-9 md:text-[10rem]"
						>
							{i + 1}
						</span>
						<!-- Focus (keyboard, TV remote) lifts the poster with a gold ring, like MediaCard. -->
						<div
							class="relative aspect-[2/3] w-[127px] shrink-0 overflow-hidden rounded-md bg-surface ring-gold transition-transform duration-200 group-hover:scale-[1.04] group-focus-visible:scale-[1.07] group-focus-visible:ring-[3px] sm:w-[168px] md:w-[192px]"
						>
							{#if !failedIds.has(item.id)}
								<img
									src={imageUrl(item.poster_url, token)}
									alt={item.title}
									loading="lazy"
									onerror={() => failedIds.add(item.id)}
									class="size-full object-cover"
								/>
							{:else}
								<p class="p-2 text-center text-xs text-neutral-400">{item.title}</p>
							{/if}
							<!-- Same badge idiom as MediaCard.svelte — see its own comment. -->
							{#if item.badge}
								<span
									class="absolute top-1.5 left-1.5 rounded bg-gold px-1.5 py-0.5 text-[0.6rem] font-semibold text-black sm:text-xs"
								>
									{t(`home.badge_${item.badge}`)}
								</span>
							{/if}
						</div>
					</a>
				{/each}
			{/if}
		</Carousel>
	</section>
{/if}
