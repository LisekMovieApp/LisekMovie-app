<!--
  Faza 8 — one person (actor, director, writer): a large profile photo on
  the left; name, biography (collapsed behind "Pokaż więcej" when long),
  birth date and place, date of death with the age, links to TMDB and
  IMDb on the right; below, carousels of the films and series in this
  server's libraries that credit them, with the part they play.

  The biography and dates are read from TMDB the first time the page is
  opened (`lisek-server::people`), so a first visit may take a moment.
  Only titles this account may see are listed; a person with none is a
  404, like a missing film. Same `$effect` + `loadSeq` pattern as
  `/media/[id]`, so moving between two people never shows the previous one.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import Carousel from '$lib/components/Carousel.svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import { imageUrl, MediaNotFoundError } from '$lib/media/api';
	import {
		fetchPerson,
		formatLifeDate,
		initials,
		isLongBiography,
		type FilmographyItem,
		type Person
	} from '$lib/media/people';

	const personId = $derived(page.params.id ?? '');

	let person = $state<Person | null>(null);
	let token = $state('');
	let loading = $state(true);
	let notFound = $state(false);
	let loadFailed = $state(false);
	let photoFailed = $state(false);
	let backdropFailed = $state(false);
	let expanded = $state(false);
	let loadSeq = 0;

	async function load(id: string, mySeq: number) {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;
		try {
			const result = await fetchPerson(sessionToken, id);
			if (mySeq !== loadSeq) return;
			person = result;
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
		photoFailed = false;
		backdropFailed = false;
		expanded = false;
		person = null;
		load(id, mySeq);
	}

	$effect(() => {
		const id = personId;
		triggerLoad(id);
	});

	const movies = $derived(person?.filmography.filter((f) => f.item.item_type === 'movie') ?? []);
	const series = $derived(person?.filmography.filter((f) => f.item.item_type === 'series') ?? []);
	/** The newest title's backdrop, faded behind the page. */
	const backdrop = $derived(person?.filmography[0]?.item.backdrop_url ?? null);
	const longBio = $derived(isLongBiography(person?.biography));
	const locale = $derived(getLocale());

	/** What the card says under the title: the part, or the crew role. */
	function caption(entry: FilmographyItem): string | null {
		if (entry.character) return entry.character;
		const crew = entry.roles.filter((r) => r !== 'Actor');
		return crew.length > 0 ? crew.map((r) => t(`people.role.${r}`)).join(', ') : null;
	}
</script>

<svelte:head>
	<title>{person ? `${person.name} · LisekMovie` : 'LisekMovie'}</title>
</svelte:head>

<main class="relative min-h-screen overflow-hidden bg-base pb-16">
	{#if loading}
		<div class="flex min-h-screen items-center justify-center">
			<p class="text-sm text-neutral-400">{t('common.loading')}</p>
		</div>
	{:else if notFound}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-neutral-400">{t('people.not_found')}</p>
			<a
				href={resolve('/')}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('detail.back_to_home')}
			</a>
		</div>
	{:else if loadFailed}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-red-400">{t('people.load_failed')}</p>
			<button
				type="button"
				onclick={() => triggerLoad(personId)}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('home.retry')}
			</button>
		</div>
	{:else if person}
		{#if backdrop && !backdropFailed}
			<div
				class="pointer-events-none absolute inset-x-0 top-0 h-[70vh] overflow-hidden"
				aria-hidden="true"
			>
				<img
					src={imageUrl(backdrop, token)}
					alt=""
					onerror={() => (backdropFailed = true)}
					class="size-full object-cover opacity-25 blur-sm"
				/>
				<div class="absolute inset-0 bg-gradient-to-b from-black/60 via-base/80 to-base"></div>
			</div>
		{/if}

		<section
			class="relative mx-auto flex max-w-6xl flex-col items-center gap-8 px-4 pt-24 md:flex-row md:items-start md:gap-12 md:px-8 md:pt-28"
			aria-labelledby="person-name"
		>
			<div
				class="w-48 shrink-0 overflow-hidden rounded-lg bg-surface shadow-[0_20px_50px_rgb(0_0_0/0.6)] ring-1 ring-white/10 sm:w-60 md:w-72"
			>
				<div class="relative aspect-[2/3]">
					{#if person.image_url && !photoFailed}
						<img
							src={imageUrl(person.image_url, token)}
							alt={person.name}
							onerror={() => (photoFailed = true)}
							class="size-full object-cover"
						/>
					{:else}
						<span
							class="absolute inset-0 grid place-items-center text-6xl font-semibold text-neutral-600"
							aria-hidden="true">{initials(person.name)}</span
						>
					{/if}
				</div>
			</div>

			<div class="w-full min-w-0 flex-1">
				<h1
					id="person-name"
					class="text-center text-3xl leading-tight font-bold text-white drop-shadow md:text-left md:text-5xl"
				>
					{person.name}
				</h1>

				{#if person.biography}
					<div class="mt-6">
						<p
							id="person-biography"
							class="text-sm leading-relaxed whitespace-pre-line text-neutral-200 md:text-[1rem] md:leading-7 {longBio &&
							!expanded
								? 'line-clamp-6'
								: ''}"
						>
							{person.biography}
						</p>
						{#if longBio}
							<button
								type="button"
								aria-expanded={expanded}
								aria-controls="person-biography"
								onclick={() => (expanded = !expanded)}
								class="mt-2 text-sm font-medium text-gold hover:underline focus-visible:outline-2"
							>
								{expanded ? t('people.show_less') : t('people.show_more')}
							</button>
						{/if}
					</div>
				{:else}
					<p class="mt-6 text-sm text-neutral-500">{t('people.no_biography')}</p>
				{/if}

				<dl class="mt-8 grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm md:text-[0.95rem]">
					{#if person.birth_date}
						<dt class="text-neutral-400">{t('people.born')}</dt>
						<dd class="text-neutral-100">
							{formatLifeDate(person.birth_date, locale)}
							{#if !person.death_date && person.age !== null && person.age !== undefined}
								<span class="text-neutral-400">{t('people.age', { age: person.age })}</span>
							{/if}
						</dd>
					{/if}
					{#if person.birth_place}
						<dt class="text-neutral-400">{t('people.birth_place')}</dt>
						<dd class="text-neutral-100">{person.birth_place}</dd>
					{/if}
					{#if person.death_date}
						<dt class="text-neutral-400">{t('people.died')}</dt>
						<dd class="text-neutral-100">
							{formatLifeDate(person.death_date, locale)}
							{#if person.age !== null && person.age !== undefined}
								<span class="text-neutral-400">{t('people.age_at_death', { age: person.age })}</span
								>
							{/if}
						</dd>
					{/if}
				</dl>

				<div class="mt-6 flex flex-wrap gap-3">
					{#if person.imdb_url}
						<!-- eslint-disable svelte/no-navigation-without-resolve -- external sites -->
						<a
							href={person.imdb_url}
							target="_blank"
							rel="noopener noreferrer"
							class="flex items-center gap-1.5 rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold hover:text-gold focus-visible:outline-2"
						>
							{t('people.imdb')}
							<Icon name="external" class="size-4" />
						</a>
					{/if}
					<a
						href={person.tmdb_url}
						target="_blank"
						rel="noopener noreferrer"
						class="flex items-center gap-1.5 rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold hover:text-gold focus-visible:outline-2"
					>
						{t('people.tmdb')}
						<Icon name="external" class="size-4" />
					</a>
					<!-- eslint-enable svelte/no-navigation-without-resolve -->
				</div>
			</div>
		</section>

		{#if movies.length > 0}
			<section class="group/row relative mt-12" aria-labelledby="person-movies">
				<h2 id="person-movies" class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">
					{t('people.movies')}
				</h2>
				<Carousel>
					{#each movies as entry (entry.item.id)}
						<MediaCard item={entry.item} {token} caption={caption(entry)} />
					{/each}
				</Carousel>
			</section>
		{/if}
		{#if series.length > 0}
			<section class="group/row relative mt-8" aria-labelledby="person-series">
				<h2 id="person-series" class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">
					{t('people.series')}
				</h2>
				<Carousel>
					{#each series as entry (entry.item.id)}
						<MediaCard item={entry.item} {token} caption={caption(entry)} />
					{/each}
				</Carousel>
			</section>
		{/if}
	{/if}
</main>
