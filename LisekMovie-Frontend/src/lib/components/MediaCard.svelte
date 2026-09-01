<!--
  One real media card inside a MediaRow's carousel — the landscape (16:9)
  counterpart of SkeletonCard, once a real item exists to show in that
  spot. Uses the backdrop image, not the poster: this card's own aspect
  ratio is landscape, and a portrait poster stretched to 16:9 would distort
  badly, unlike a backdrop which is already landscape source material.

  `alt=""` is deliberate: the title overlay below already carries the same
  information as visible text, so a screen reader announcing the image
  itself would just repeat it.

  A link, not a plain div — navigates to this item's Detail View
  (`/media/[id]`, see that route's own doc comment). `block` is required
  precisely because it's now an `<a>`: an inline element ignores an
  explicit `width`, which is how this card actually gets its size.

  Runda 8: the top-left badge is `item.badge` as-is (`'new_series'` /
  `'new_season'` / `'new_episodes'`), translated — see
  `lisek_db::classify_series_badge`'s doc comment for what decides which
  one a series card gets, and why a movie card never has one at all.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { imageUrl, type MediaItem } from '$lib/media/api';

	interface Props {
		item: MediaItem;
		token: string;
	}

	let { item, token }: Props = $props();

	let imageFailed = $state(false);
</script>

<a
	href={resolve('/media/[id]', { id: item.id })}
	class="group relative block aspect-video w-[160px] shrink-0 overflow-hidden rounded-md bg-surface sm:w-[200px] md:w-[240px]"
>
	{#if !imageFailed}
		<img
			src={imageUrl(item.backdrop_url, token)}
			alt=""
			loading="lazy"
			onerror={() => (imageFailed = true)}
			class="size-full object-cover transition-transform duration-300 group-hover:scale-105"
		/>
	{/if}
	{#if item.badge}
		<span
			class="absolute top-1.5 left-1.5 rounded bg-gold px-1.5 py-0.5 text-[0.6rem] font-semibold text-black sm:text-xs"
		>
			{t(`home.badge_${item.badge}`)}
		</span>
	{/if}
	<div
		class="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/85 via-black/40 to-transparent p-2 pt-6"
	>
		<p class="truncate text-xs font-medium text-neutral-100 sm:text-sm">{item.title}</p>
	</div>
</a>
