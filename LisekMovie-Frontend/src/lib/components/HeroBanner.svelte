<!--
  The big banner at the top of Home. Faza 4 proved the layout with a
  skeleton (background gradient, bottom vignette, title/metadata/button
  placement); Faza 5 wires in `item` — the backend's pick for "the one
  thing to feature" (see `lisek-db::get_hero_item`'s doc comment for how
  it's chosen). `item` stays `null` for as long as nothing has been scanned
  yet (or the feed hasn't loaded), which keeps rendering the exact same
  skeleton as before rather than a broken or empty banner.

  Play/More Info stay disabled either way: there's no playback pipeline
  yet (a later phase) for Play to start, and no per-item detail page yet
  for More Info to open — real data existing for the title/overview
  doesn't mean those buttons have anywhere real to go yet.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { imageUrl, type MediaItem } from '$lib/media/api';

	interface Props {
		item: MediaItem | null;
		token: string;
	}

	let { item, token }: Props = $props();

	// Tracks *which item id* last failed to load its backdrop, rather than a
	// plain failed/not-failed flag — so a previous item's broken-image state
	// can never carry over and incorrectly suppress a new item's backdrop
	// after a home-feed reload swaps which item is featured. No `$effect`
	// needed to reset it: once `item` changes, this id simply stops matching.
	let failedForId: string | null = $state(null);
	let backdropFailed = $derived(item !== null && failedForId === item.id);

	/** A movie shows its year; an episode shows "S{season}E{episode}" — more
	 * useful here than a generic "TV Shows" label repeated on every card. */
	let subtitle = $derived.by(() => {
		if (!item) return '';
		if (item.item_type === 'episode' && item.season !== null && item.episode !== null) {
			return `S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`;
		}
		return item.year ? String(item.year) : '';
	});
</script>

<section class="relative h-[80vh] max-h-[760px] min-h-[420px] w-full overflow-hidden bg-surface">
	{#if item && !backdropFailed}
		<img
			src={imageUrl(item.backdrop_url, token)}
			alt=""
			onerror={() => (failedForId = item?.id ?? null)}
			class="absolute inset-0 size-full object-cover"
		/>
	{:else}
		<!-- No item yet, or no cached backdrop for it: a brand-tinted gradient stands in. -->
		<div class="absolute inset-0 bg-gradient-to-br from-surface via-black to-surface"></div>
		<div
			class="absolute inset-0 bg-[radial-gradient(ellipse_at_30%_20%,rgba(189,162,77,0.14),transparent_60%)]"
		></div>
	{/if}

	<!-- Top scrim, keeps the transparent navbar's logo/links readable over any background. -->
	<div class="absolute inset-x-0 top-0 h-32 bg-gradient-to-b from-black/70 to-transparent"></div>

	<!-- Bottom vignette — the winieta that blends the banner into the page background. -->
	<div
		class="absolute inset-x-0 bottom-0 h-2/3 bg-gradient-to-t from-base via-base/70 to-transparent"
	></div>

	<div class="absolute bottom-0 left-0 flex w-full max-w-xl flex-col gap-4 p-4 md:p-8 lg:p-12">
		{#if item}
			<h1 class="text-3xl leading-tight font-bold text-white drop-shadow md:text-5xl">
				{item.title}
			</h1>
			{#if subtitle}
				<p class="text-sm text-neutral-300">{subtitle}</p>
			{/if}
			{#if item.overview}
				<p class="line-clamp-3 max-w-lg text-sm text-white drop-shadow-md md:text-base">
					{item.overview}
				</p>
			{/if}
		{:else}
			<div class="flex flex-col gap-2" aria-hidden="true">
				<div class="h-8 w-3/4 animate-pulse rounded bg-white/20 md:h-10"></div>
				<div class="h-8 w-1/2 animate-pulse rounded bg-white/20 md:h-10"></div>
			</div>
			<div class="h-4 w-48 animate-pulse rounded bg-white/10" aria-hidden="true"></div>
		{/if}

		<div class="mt-2 flex gap-3">
			<button
				type="button"
				disabled
				class="rounded-md bg-gold px-6 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
			>
				{t('home.play')}
			</button>
			<button
				type="button"
				disabled
				class="rounded-md bg-white/20 px-6 py-2.5 font-medium text-white backdrop-blur-sm transition-colors hover:bg-white/30 disabled:cursor-not-allowed disabled:opacity-50"
			>
				{t('home.more_info')}
			</button>
		</div>
	</div>
</section>
