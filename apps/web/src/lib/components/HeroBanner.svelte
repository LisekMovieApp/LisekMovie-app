<!--
  The big banner at the top of Home. Faza 4 proved the layout with a
  skeleton (background gradient, bottom vignette, title/metadata/button
  placement); Faza 5 wires in `item` — the backend's pick for "the one
  thing to feature" (see `lisek-db::get_hero_item`'s doc comment for how
  it's chosen). `item` stays `null` for as long as nothing has been scanned
  yet (or the feed hasn't loaded), which keeps rendering the exact same
  skeleton as before rather than a broken or empty banner.

  Play stays disabled either way: there's no playback pipeline yet (a
  later phase) for it to start. "More Info" is now a real link to this
  item's Detail View (`/media/[id]`) once `item` exists — it only stays a
  disabled placeholder button while `item` is still `null` (the skeleton
  loading state has nowhere to link to yet).

  The title is a logo image (`item.logo_url`) when the matched provider
  actually had one, falling back to the plain-text `<h1>` otherwise — same
  "try the image, fall back to text on load failure" idiom this component
  already uses for the backdrop itself, tracked the same way (by item id,
  not a plain flag, so a previous item's failure can't leak onto a new one).

  Both branches share one `min-h-20 md:min-h-32` wrapper (`items-end`) so
  swapping between them — a 404'd logo falling back to text, or a
  home-feed reload changing which item is featured — never visibly jumps
  the subtitle/button row underneath it. Without a shared reserved height,
  the two branches' naturally different heights make that swap read as
  "the layout just broke" for the instant it happens — the exact bug
  reported against this same pattern in the Detail View (see that page's
  own comment for the full story).
-->
<script lang="ts">
	import { resolve } from '$app/paths';
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

	// Same by-id tracking as `failedForId` above, for the logo image.
	let logoFailedForId: string | null = $state(null);
	let logoFailed = $derived(item !== null && logoFailedForId === item.id);

	/** A movie shows its year; a series shows nothing here — there's no
	 * single year for a whole show (see `lisek_db`'s `home_candidates` CTE
	 * doc comment on why `year` is always `null` for a `'series'` item). */
	let subtitle = $derived.by(() => {
		if (!item) return '';
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
			<div class="flex min-h-20 items-end md:min-h-32">
				{#if item.logo_url && !logoFailed}
					<img
						src={imageUrl(item.logo_url, token)}
						alt={item.title}
						onerror={() => (logoFailedForId = item?.id ?? null)}
						class="max-h-20 max-w-[85%] object-contain object-left drop-shadow-md md:max-h-32"
					/>
				{:else}
					<h1 class="text-3xl leading-tight font-bold text-white drop-shadow md:text-5xl">
						{item.title}
					</h1>
				{/if}
			</div>
			{#if subtitle}
				<p class="text-sm text-neutral-300">{subtitle}</p>
			{/if}
			{#if item.overview}
				<!--
				  `md:text-[1rem]` on purpose, not `md:text-base`: this theme
				  defines `--color-base` (our near-black bg token, layout.css),
				  and Tailwind v4 resolves a bare `text-base` utility against
				  that color-scale entry instead of the built-in font-size
				  scale entry of the same name — so `md:text-base` compiled to
				  `color: var(--color-base)` (near-black) at the md breakpoint
				  only, with no font-size rule at all. Exactly why this text
				  was white on mobile (text-sm, no md: override in play) and
				  unreadable on desktop. The arbitrary value bypasses theme-key
				  lookup entirely, so it can only ever mean font-size; paired
				  with `md:leading-6` (1.5rem) to match text-base's own
				  default line-height. Any other spot that ever needs the
				  *size* "base" (not the color) needs the same arbitrary-value
				  treatment — literally `text-base` is claimed by the color
				  token project-wide.
				-->
				<p
					class="line-clamp-3 max-w-lg text-sm text-white drop-shadow-md md:text-[1rem] md:leading-6"
				>
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
			{#if item}
				<a
					href={resolve('/media/[id]', { id: item.id })}
					class="rounded-md bg-white/20 px-6 py-2.5 font-medium text-white backdrop-blur-sm transition-colors hover:bg-white/30"
				>
					{t('home.more_info')}
				</a>
			{:else}
				<button
					type="button"
					disabled
					class="rounded-md bg-white/20 px-6 py-2.5 font-medium text-white backdrop-blur-sm transition-colors hover:bg-white/30 disabled:cursor-not-allowed disabled:opacity-50"
				>
					{t('home.more_info')}
				</button>
			{/if}
		</div>
	</div>
</section>
