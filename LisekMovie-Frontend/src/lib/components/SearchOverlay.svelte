<!--
  The omnibox (Faza 11): a full-screen layer with one large search field
  at the top and, under it, the results as VOD rows — Filmy and Seriale
  (posters), Odcinki (16:9 stills, "S2:E5 - Tytuł"), Osoby (profile
  cards), Kolekcje. Typing waits `DEBOUNCE_MS` after the last key and
  cancels a request a newer query made stale.

  TV remote: the layer is a spatial-navigation scope with the field
  focused; ↓ leaves the field into the first row of results, every row is
  a `Carousel` (left/right inside it, back to the card left last). Back,
  Escape or the close button leaves; following a result closes the layer.
-->
<script lang="ts">
	import { beforeNavigate } from '$app/navigation';
	import { t } from '$lib/i18n';
	import {
		DEBOUNCE_MS,
		hasResults,
		isSearchable,
		searchCatalog,
		type SearchResults
	} from '$lib/media/search';
	import { navScope } from '$lib/spatial/spatial';
	import Carousel from './Carousel.svelte';
	import EpisodeCard from './EpisodeCard.svelte';
	import Icon from './Icon.svelte';
	import MediaCard from './MediaCard.svelte';
	import PersonCard from './PersonCard.svelte';
	import PosterCard from './PosterCard.svelte';

	interface Props {
		token: string;
		onclose: () => void;
	}

	let { token, onclose }: Props = $props();

	let query = $state('');
	let results = $state<SearchResults | null>(null);
	let loading = $state(false);
	let failed = $state(false);

	/** Debounced search: a new keystroke restarts the wait and cancels the
	 * request still in flight. */
	$effect(() => {
		const text = query.trim();
		failed = false;
		if (!isSearchable(text)) {
			results = null;
			loading = false;
			return;
		}
		loading = true;
		const controller = new AbortController();
		const timer = setTimeout(() => {
			searchCatalog(token, text, controller.signal)
				.then((found) => {
					results = found;
					loading = false;
				})
				.catch((err: unknown) => {
					if (controller.signal.aborted) return;
					void err;
					failed = true;
					loading = false;
				});
		}, DEBOUNCE_MS);
		return () => {
			clearTimeout(timer);
			controller.abort();
		};
	});

	// A result was followed: the page behind changes, the layer goes.
	// (`beforeNavigate`: `afterNavigate` also fires when the layer mounts.)
	beforeNavigate(() => onclose());

	$effect(() => {
		const previous = document.body.style.overflow;
		document.body.style.overflow = 'hidden';
		const onBack = (event: Event) => {
			event.preventDefault();
			onclose();
		};
		window.addEventListener('spatialback', onBack);
		return () => {
			document.body.style.overflow = previous;
			window.removeEventListener('spatialback', onBack);
		};
	});

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			onclose();
		}
	}

	const sections = $derived(
		results
			? [
					{ key: 'movies', titleKey: 'search.movies', count: results.movies.length },
					{ key: 'series', titleKey: 'search.series', count: results.series.length },
					{ key: 'episodes', titleKey: 'search.episodes', count: results.episodes.length },
					{ key: 'people', titleKey: 'search.people', count: results.people.length },
					{ key: 'collections', titleKey: 'search.collections', count: results.collections.length }
				].filter((s) => s.count > 0)
			: []
	);
</script>

<svelte:window onkeydown={onKeydown} />

<div
	class="fixed inset-0 z-[60] overflow-y-auto bg-base/[0.97] pb-16 backdrop-blur-md"
	role="dialog"
	aria-modal="true"
	aria-label={t('search.title')}
	use:navScope={{ autofocus: true }}
>
	<div
		class="sticky top-0 z-10 bg-gradient-to-b from-base via-base/95 to-transparent px-4 pt-5 pb-8 md:px-8 md:pt-8"
	>
		<div class="mx-auto flex max-w-4xl items-center gap-3">
			<label class="relative flex-1">
				<span class="sr-only">{t('search.title')}</span>
				<Icon
					name="search"
					class="pointer-events-none absolute top-1/2 left-4 size-6 -translate-y-1/2 text-neutral-400 md:left-5 md:size-7"
				/>
				<input
					bind:value={query}
					data-nav-default
					type="search"
					inputmode="search"
					enterkeyhint="search"
					autocomplete="off"
					spellcheck="false"
					maxlength="100"
					placeholder={t('search.placeholder')}
					class="w-full rounded-xl border border-white/15 bg-white/[0.06] py-3.5 pr-4 pl-13 text-lg text-white placeholder:text-neutral-500 focus:border-gold focus:outline-none md:py-5 md:pl-16 md:text-3xl [&::-webkit-search-cancel-button]:appearance-none"
				/>
			</label>
			<button
				type="button"
				onclick={onclose}
				aria-label={t('search.close')}
				title={t('search.close')}
				class="grid size-12 shrink-0 place-items-center rounded-full text-neutral-200 hover:bg-white/10 hover:text-gold focus-visible:bg-white/15 focus-visible:text-gold focus-visible:outline-2 focus-visible:outline-gold"
			>
				<Icon name="close" class="size-7" />
			</button>
		</div>
		<p class="mx-auto mt-3 h-5 max-w-4xl text-sm text-neutral-500" aria-live="polite">
			{#if loading}
				{t('search.searching')}
			{:else if failed}
				<span class="text-red-300">{t('search.failed')}</span>
			{:else if !isSearchable(query)}
				{t('search.hint')}
			{:else if results && !hasResults(results)}
				{t('search.nothing', { query: results.query })}
			{/if}
		</p>
	</div>

	{#if results && hasResults(results)}
		<div class="flex flex-col gap-8">
			{#each sections as section (section.key)}
				<section class="group/row relative" aria-labelledby="search-{section.key}">
					<h2
						id="search-{section.key}"
						class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8"
					>
						{t(section.titleKey)}
						<span class="ml-1 text-sm font-normal text-neutral-500">{section.count}</span>
					</h2>
					<Carousel>
						{#if section.key === 'movies'}
							{#each results.movies as item (item.id)}
								<PosterCard {item} {token} />
							{/each}
						{:else if section.key === 'series'}
							{#each results.series as item (item.id)}
								<PosterCard {item} {token} />
							{/each}
						{:else if section.key === 'episodes'}
							{#each results.episodes as episode (episode.id)}
								<EpisodeCard {episode} {token} />
							{/each}
						{:else if section.key === 'people'}
							{#each results.people as person (person.person_id)}
								<PersonCard credit={person} {token} />
							{/each}
						{:else}
							{#each results.collections as item (item.id)}
								<MediaCard {item} {token} />
							{/each}
						{/if}
					</Carousel>
				</section>
			{/each}
		</div>
	{/if}
</div>
