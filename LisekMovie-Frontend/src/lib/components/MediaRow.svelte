<!--
  A horizontally-sliding row of media: a title plus a `Carousel` strip of
  cards. `items` is `null` while the home feed hasn't loaded yet (renders
  `SkeletonCard` placeholders, same look Faza 4 shipped); once loaded, an
  empty row (a real library with nothing matching this row's query — e.g.
  a brand new library before its first scan) renders nothing at all rather
  than an empty heading with no cards under it. The sliding mechanics
  themselves live in `Carousel.svelte` — see its own comment for why this
  is CSS-transform-based rather than native-scroll-based.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import Carousel from './Carousel.svelte';
	import SkeletonCard from './SkeletonCard.svelte';
	import MediaCard from './MediaCard.svelte';
	import type { MediaItem } from '$lib/media/api';

	interface Props {
		/** i18n key for the row's heading, e.g. "home.row_recently_added". */
		titleKey: string;
		/** Runda 10: optional interpolation params for `titleKey` (e.g.
		 * `{ name: library.name }` for `/library/[id]`'s dynamic
		 * "Ostatnio dodane — {name}" style headings) — `t()` already
		 * supports this (see `admin.scan_result`'s usage elsewhere), this
		 * component just wasn't threading it through yet. Omitted by every
		 * existing Home call site, so their plain, param-free `titleKey`
		 * lookups are unaffected. */
		titleParams?: Record<string, string | number>;
		/** `null` = still loading (renders skeletons); `[]` = loaded and empty (renders nothing). */
		items: MediaItem[] | null;
		token: string;
		/** How many skeleton cards to render while loading. */
		skeletonCount?: number;
	}

	let { titleKey, titleParams, items, token, skeletonCount = 10 }: Props = $props();
</script>

{#if items === null || items.length > 0}
	<section class="group/row relative">
		<h2 class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">
			{t(titleKey, titleParams)}
		</h2>

		<Carousel>
			{#if items === null}
				{#each { length: skeletonCount } as _, i (i)}
					<SkeletonCard />
				{/each}
			{:else}
				{#each items as item (item.id)}
					<MediaCard {item} {token} />
				{/each}
			{/if}
		</Carousel>
	</section>
{/if}
