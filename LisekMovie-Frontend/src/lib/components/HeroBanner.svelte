<!--
  Home hero (Faza 4, VOD style): the featured title's backdrop fills the
  top of the page as a slowly drifting background, and the banner rotates
  through a few featured items (the feed's hero first, then the top
  recommendations) with a cross-fade. Rotation pauses while the pointer or
  focus is inside the banner, while the tab is hidden, on request (the
  pause button) and entirely under `prefers-reduced-motion`. The rows
  below overlap the banner's bottom fade (see the Home page).
-->
<script lang="ts">
	import { SvelteSet } from 'svelte/reactivity';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { imageUrl, type MediaItem } from '$lib/media/api';
	import { navGroup } from '$lib/spatial/spatial';
	import Icon from './Icon.svelte';

	interface Props {
		/** Featured items, first shown first; `null` while the feed loads. */
		items: MediaItem[] | null;
		token: string;
	}

	let { items, token }: Props = $props();

	const ROTATE_MS = 9000;

	let index = $state(0);
	let hovered = $state(false);
	let focused = $state(false);
	let stopped = $state(false);
	let hidden = $state(false);
	const reduced =
		typeof window !== 'undefined' &&
		window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

	const slides = $derived(items ?? []);
	const item = $derived(slides.length ? slides[Math.min(index, slides.length - 1)] : null);
	const rotating = $derived(slides.length > 1 && !reduced && !stopped);

	// Tracks *which item ids* failed to load their backdrop or logo, rather
	// than a plain failed/not-failed flag — so one item's broken image can
	// never suppress another item's after the banner rotates or a home-feed
	// reload swaps which items are featured.
	const failedBackdrops = new SvelteSet<string>();
	const failedLogos = new SvelteSet<string>();

	/** A movie shows its year; a series shows nothing here — there's no
	 * single year for a whole show (see `lisek_db`'s `home_candidates` CTE
	 * doc comment on why `year` is always `null` for a `'series'` item). */
	let subtitle = $derived.by(() => {
		if (!item) return '';
		const kind = t(item.item_type === 'series' ? 'home.type_series' : 'home.type_movie');
		return item.year ? `${item.year} · ${kind}` : kind;
	});

	$effect(() => {
		if (!rotating || hovered || focused || hidden) return;
		const count = slides.length;
		const timer = setInterval(() => (index = (index + 1) % count), ROTATE_MS);
		return () => clearInterval(timer);
	});

	$effect(() => {
		const onVisibility = () => (hidden = document.visibilityState === 'hidden');
		document.addEventListener('visibilitychange', onVisibility);
		return () => document.removeEventListener('visibilitychange', onVisibility);
	});

	$effect(() => {
		// A reloaded feed with fewer items must not leave the index past the end.
		if (index >= slides.length) index = 0;
	});

	function playHref(target: MediaItem) {
		return resolve(target.item_type === 'series' ? '/media/[id]' : '/watch/[id]', {
			id: target.id
		});
	}
</script>

<section
	class="relative h-[78vh] max-h-[900px] min-h-[460px] w-full overflow-hidden bg-surface"
	aria-roledescription="carousel"
	aria-label={t('home.featured')}
	onmouseenter={() => (hovered = true)}
	onmouseleave={() => (hovered = false)}
	onfocusin={() => (focused = true)}
	onfocusout={() => (focused = false)}
	use:navGroup
>
	{#each slides as slide, i (slide.id)}
		<div
			class="absolute inset-0 transition-opacity duration-[1200ms] ease-out {i === index
				? 'opacity-100'
				: 'opacity-0'}"
			aria-hidden="true"
		>
			{#if !failedBackdrops.has(slide.id)}
				<img
					src={imageUrl(slide.backdrop_url, token)}
					alt=""
					loading={i === 0 ? 'eager' : 'lazy'}
					fetchpriority={i === 0 ? 'high' : 'auto'}
					onerror={() => failedBackdrops.add(slide.id)}
					class="size-full object-cover object-[center_25%] {i === index && !reduced
						? 'drift'
						: ''}"
				/>
			{/if}
		</div>
	{/each}

	{#if !item || failedBackdrops.has(item.id)}
		<!-- No item yet, or no cached backdrop for it: a brand-tinted gradient stands in. -->
		<div class="absolute inset-0 bg-gradient-to-br from-surface via-black to-surface"></div>
		<div
			class="absolute inset-0 bg-[radial-gradient(ellipse_at_30%_20%,rgba(189,162,77,0.14),transparent_60%)]"
		></div>
	{/if}

	<!-- Top scrim, keeps the transparent navbar's logo/links readable over any background. -->
	<div class="absolute inset-x-0 top-0 h-36 bg-gradient-to-b from-black/75 to-transparent"></div>
	<!-- Left scrim behind the title and description. -->
	<div
		class="absolute inset-y-0 left-0 w-full bg-gradient-to-r from-black/85 via-black/35 to-transparent md:w-2/3"
	></div>
	<!-- Bottom vignette — the winieta that blends the banner into the page background. -->
	<div
		class="absolute inset-x-0 bottom-0 h-1/2 bg-gradient-to-t from-base via-base/60 to-transparent"
	></div>

	<div
		class="absolute bottom-[16%] left-0 flex w-full max-w-2xl flex-col gap-4 px-4 md:bottom-[30%] md:px-8 lg:px-12"
	>
		{#if item}
			{#key item.id}
				<div class="hero-in flex flex-col gap-4">
					<div class="flex min-h-20 items-end md:min-h-32">
						{#if item.logo_url && !failedLogos.has(item.id)}
							<img
								src={imageUrl(item.logo_url, token)}
								alt={item.title}
								onerror={() => failedLogos.add(item.id)}
								class="max-h-20 max-w-[85%] object-contain object-left drop-shadow-[0_4px_12px_rgb(0_0_0/0.6)] md:max-h-36"
							/>
						{:else}
							<h1
								class="text-4xl leading-[1.05] font-extrabold tracking-tight text-white drop-shadow-lg md:text-6xl"
							>
								{item.title}
							</h1>
						{/if}
					</div>
					{#if subtitle}
						<p class="text-sm font-medium text-neutral-200 drop-shadow md:text-[1rem]">
							{subtitle}
						</p>
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
							class="line-clamp-3 max-w-xl text-sm text-neutral-100 drop-shadow-md md:text-[1.05rem] md:leading-7"
						>
							{item.overview}
						</p>
					{/if}
				</div>
			{/key}
		{:else}
			<div class="flex flex-col gap-2" aria-hidden="true">
				<div class="h-8 w-3/4 animate-pulse rounded bg-white/20 md:h-10"></div>
				<div class="h-8 w-1/2 animate-pulse rounded bg-white/20 md:h-10"></div>
			</div>
			<div class="h-4 w-48 animate-pulse rounded bg-white/10" aria-hidden="true"></div>
		{/if}

		<div class="mt-2 flex flex-wrap gap-2 md:gap-3">
			{#if item}
				<a
					href={playHref(item)}
					data-nav-default
					class="flex items-center gap-2 rounded-md bg-gold px-4 py-3 font-semibold text-black shadow-lg transition hover:bg-gold-dim focus-visible:scale-105 focus-visible:outline-white sm:px-6 md:px-8 md:text-lg"
				>
					<Icon name="play" class="size-6" />
					{t('home.play')}
				</a>
				<a
					href={resolve('/media/[id]', { id: item.id })}
					class="flex items-center gap-2 rounded-md bg-white/20 px-4 py-3 font-semibold text-white backdrop-blur-md transition hover:bg-white/30 focus-visible:scale-105 sm:px-6 md:px-8 md:text-lg"
				>
					<Icon name="info" class="size-6" />
					{t('home.more_info')}
				</a>
			{:else}
				<button
					type="button"
					disabled
					class="rounded-md bg-gold px-6 py-3 font-semibold text-black disabled:cursor-not-allowed disabled:opacity-50"
				>
					{t('home.play')}
				</button>
				<button
					type="button"
					disabled
					class="rounded-md bg-white/20 px-6 py-3 font-semibold text-white backdrop-blur-sm disabled:cursor-not-allowed disabled:opacity-50"
				>
					{t('home.more_info')}
				</button>
			{/if}
		</div>
	</div>

	{#if slides.length > 1}
		<!-- Phones: top right, clear of the buttons; larger screens: beside them. -->
		<div
			class="absolute top-20 right-4 flex items-center gap-2 md:top-auto md:right-12 md:bottom-[30%]"
		>
			{#if !reduced}
				<button
					type="button"
					class="mr-1 grid size-8 place-items-center rounded-full text-white/80 ring-1 ring-white/30 hover:text-white"
					aria-label={stopped ? t('home.hero_resume') : t('home.hero_pause')}
					onclick={() => (stopped = !stopped)}
				>
					<Icon name={stopped ? 'play' : 'pause'} class="size-3.5" />
				</button>
			{/if}
			{#each slides as slide, i (slide.id)}
				<button
					type="button"
					class="h-1.5 rounded-full transition-all duration-300 {i === index
						? 'w-8 bg-gold'
						: 'w-3 bg-white/40 hover:bg-white/70'}"
					aria-label={t('home.hero_show', { title: slide.title })}
					aria-current={i === index ? 'true' : undefined}
					onclick={() => (index = i)}
				></button>
			{/each}
		</div>
	{/if}
</section>

<style>
	/* Slow drift of the featured backdrop (Ken Burns), restarted per slide. */
	.drift {
		animation: drift 14s ease-out both;
	}
	@keyframes drift {
		from {
			transform: scale(1.12) translate3d(1.5%, 1%, 0);
		}
		to {
			transform: scale(1.02) translate3d(0, 0, 0);
		}
	}
	.hero-in {
		animation: hero-in 700ms ease-out both;
	}
	@keyframes hero-in {
		from {
			opacity: 0;
			transform: translateY(12px);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.hero-in {
			animation: none;
		}
	}
</style>
