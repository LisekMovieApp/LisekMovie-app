<!--
  A portrait (2:3) card for a film, series or collection: the poster, the
  title and year underneath, linking to the detail page (a collection to
  `/collection/[id]`). Faza 11 made it for search results; Faza 12 uses
  it for the library grid too (`fluid`: the grid sets the width) with the
  same "new" and collection badges as MediaCard.
  Focus (keyboard, TV remote) lifts it with a gold ring, like MediaCard.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { imageUrl, type MediaItem } from '$lib/media/api';
	import Icon from './Icon.svelte';

	interface Props {
		item: MediaItem;
		token: string;
		/** Fill the parent's width (a grid cell) instead of a fixed size. */
		fluid?: boolean;
	}

	let { item, token, fluid = false }: Props = $props();
	let failed = $state(false);
	const isCollection = $derived(item.item_type === 'collection');
	const target = $derived(
		isCollection
			? resolve('/collection/[id]', { id: item.id })
			: resolve('/media/[id]', { id: item.id })
	);
</script>

<a
	href={target}
	aria-label={isCollection
		? t('collections.card_label', { name: item.title, count: item.item_count ?? 0 })
		: undefined}
	class="group block shrink-0 outline-none {fluid
		? 'w-full min-w-0'
		: 'w-[112px] sm:w-[132px] md:w-[152px]'}"
>
	<div
		class="relative aspect-[2/3] overflow-hidden rounded-md bg-surface ring-gold transition-[transform,box-shadow] duration-200 group-hover:shadow-[0_10px_30px_rgb(0_0_0/0.6)] group-focus-visible:scale-[1.05] group-focus-visible:shadow-[0_10px_30px_rgb(0_0_0/0.7)] group-focus-visible:ring-[3px]"
	>
		{#if !failed}
			<img
				src={imageUrl(item.poster_url, token)}
				alt=""
				loading="lazy"
				onerror={() => (failed = true)}
				class="size-full object-cover transition-transform duration-300 group-hover:scale-105"
			/>
		{:else}
			<span
				class="absolute inset-0 grid place-items-center p-2 text-center text-sm text-neutral-400"
				>{item.title}</span
			>
		{/if}
		{#if isCollection}
			<span
				class="absolute top-1.5 left-1.5 flex items-center gap-1 rounded bg-black/75 px-1.5 py-0.5 text-[0.6rem] font-semibold text-gold ring-1 ring-gold/40 backdrop-blur sm:text-xs"
			>
				<Icon name="stack" class="size-3 sm:size-3.5" />
				{t('collections.badge', { count: item.item_count ?? 0 })}
			</span>
		{:else if item.badge}
			<span
				class="absolute top-1.5 left-1.5 rounded bg-gold px-1.5 py-0.5 text-[0.6rem] font-semibold text-black sm:text-xs"
			>
				{t(`home.badge_${item.badge}`)}
			</span>
		{/if}
	</div>
	<p class="mt-2 truncate text-sm font-medium text-neutral-100 group-focus-visible:text-gold">
		{item.title}
	</p>
	{#if item.year}
		<p class="text-xs text-neutral-500">{item.year}</p>
	{/if}
</a>
