<!--
  Faza 6 — one collection ("box set"): its backdrop, name, year span and
  film count, "Odtwórz od początku" (the first film), then every film in
  release order as poster tiles leading to each film's own Detail View.

  Only the films this account may see are listed (parental controls,
  `lisek-server::collections`); a collection with none is a 404 here like
  a missing film. Same `$effect` + `loadSeq` pattern as `/media/[id]`, so
  moving between two collections never shows the previous one.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { navGroup } from '$lib/spatial/spatial';
	import Icon from '$lib/components/Icon.svelte';
	import {
		fetchCollection,
		imageUrl,
		MediaNotFoundError,
		yearSpan,
		type Collection
	} from '$lib/media/api';

	const collectionId = $derived(page.params.id ?? '');

	let collection = $state<Collection | null>(null);
	let token = $state('');
	let loading = $state(true);
	let notFound = $state(false);
	let loadFailed = $state(false);
	let backdropFailed = $state(false);
	let failedPosters = $state<Record<string, boolean>>({});
	let loadSeq = 0;

	async function load(id: string, mySeq: number) {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;
		try {
			const result = await fetchCollection(sessionToken, id);
			if (mySeq !== loadSeq) return;
			collection = result;
		} catch (err) {
			if (mySeq !== loadSeq) return;
			if (err instanceof MediaNotFoundError) notFound = true;
			else loadFailed = true;
		} finally {
			if (mySeq === loadSeq) loading = false;
		}
	}

	function triggerLoad(id: string) {
		const mySeq = ++loadSeq;
		loading = true;
		notFound = false;
		loadFailed = false;
		backdropFailed = false;
		failedPosters = {};
		collection = null;
		load(id, mySeq);
	}

	$effect(() => {
		const id = collectionId;
		triggerLoad(id);
	});

	const years = $derived(collection ? yearSpan(collection.first_year, collection.last_year) : '');
	const first = $derived(collection?.items[0] ?? null);
</script>

<svelte:head>
	<title>{collection ? `${collection.name} · LisekMovie` : 'LisekMovie'}</title>
</svelte:head>

<main class="min-h-screen bg-base pb-16">
	{#if loading}
		<div class="flex min-h-screen items-center justify-center">
			<p class="text-sm text-neutral-400">{t('common.loading')}</p>
		</div>
	{:else if notFound}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-neutral-400">{t('collections.not_found')}</p>
			<a
				href={resolve('/')}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('detail.back_to_home')}
			</a>
		</div>
	{:else if loadFailed}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-red-400">{t('collections.load_failed')}</p>
			<button
				type="button"
				onclick={() => triggerLoad(collectionId)}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('home.retry')}
			</button>
		</div>
	{:else if collection}
		<section
			class="relative h-[60vh] max-h-[600px] min-h-[340px] w-full overflow-hidden bg-surface"
		>
			{#if !backdropFailed}
				<img
					src={imageUrl(collection.backdrop_url, token)}
					alt=""
					onerror={() => (backdropFailed = true)}
					class="absolute inset-0 size-full object-cover"
				/>
			{:else}
				<div class="absolute inset-0 bg-gradient-to-br from-surface via-black to-surface"></div>
			{/if}
			<div
				class="absolute inset-x-0 top-0 h-32 bg-gradient-to-b from-black/70 to-transparent"
			></div>
			<div
				class="absolute inset-x-0 bottom-0 h-3/4 bg-gradient-to-t from-base via-base/70 to-transparent"
			></div>

			<div
				class="absolute bottom-0 left-0 flex w-full max-w-3xl flex-col gap-3 p-4 md:p-8 lg:p-12"
			>
				<p
					class="flex items-center gap-1.5 text-xs font-semibold tracking-[0.14em] text-gold uppercase"
				>
					<Icon name="stack" class="size-4" />
					{t('collections.kicker')}
				</p>
				<h1 class="text-3xl leading-tight font-bold text-white drop-shadow md:text-5xl">
					{collection.name}
				</h1>
				<p class="text-sm text-neutral-300">
					{years ? `${years} · ` : ''}{t('collections.film_count', {
						count: collection.item_count
					})}
				</p>
				{#if first}
					<div class="mt-2 flex gap-3">
						<a
							href={resolve('/watch/[id]', { id: first.id })}
							class="flex items-center gap-2 rounded-md bg-gold px-6 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim"
						>
							<Icon name="play" class="size-5" />
							{t('collections.play_from_start')}
						</a>
					</div>
				{/if}
			</div>
		</section>

		<div class="px-4 pt-6 md:px-8 lg:px-12">
			<h2 class="mb-4 text-lg font-semibold text-neutral-100">{t('collections.films_title')}</h2>
			<ol
				class="grid max-w-7xl grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6"
				use:navGroup
			>
				{#each collection.items as film, index (film.id)}
					<li>
						<a
							href={resolve('/media/[id]', { id: film.id })}
							class="group block rounded-md ring-gold outline-none focus-visible:ring-[3px]"
						>
							<div
								class="relative aspect-[2/3] overflow-hidden rounded-md bg-surface shadow-lg transition-transform duration-200 group-hover:scale-[1.03] group-focus-visible:scale-[1.03]"
							>
								{#if !failedPosters[film.id]}
									<img
										src={imageUrl(film.poster_url, token)}
										alt=""
										loading="lazy"
										onerror={() => (failedPosters = { ...failedPosters, [film.id]: true })}
										class="size-full object-cover"
									/>
								{:else}
									<div class="grid size-full place-items-center p-3 text-center">
										<span class="text-sm text-neutral-300">{film.title}</span>
									</div>
								{/if}
								<span
									class="absolute top-1.5 left-1.5 grid size-6 place-items-center rounded-full bg-black/70 text-xs font-bold text-gold tabular-nums"
									aria-hidden="true">{index + 1}</span
								>
							</div>
							<p class="mt-2 truncate text-sm font-medium text-neutral-100">{film.title}</p>
							{#if film.year}
								<p class="text-xs text-neutral-400">{film.year}</p>
							{/if}
						</a>
					</li>
				{/each}
			</ol>
		</div>
	{/if}
</main>
