<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchMediaDetail,
		fetchSeriesSeasons,
		fetchItemCollection,
		imageUrl,
		ageLabel,
		MediaNotFoundError,
		type Collection,
		type MediaItem,
		type SeriesSeason
	} from '$lib/media/api';
	import Carousel from '$lib/components/Carousel.svelte';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import { fetchProviders, type ProviderConfig } from '$lib/admin/providers';
	import MediaActions from '$lib/components/MediaActions.svelte';
	import MediaItemEditorModal from '$lib/components/MediaItemEditorModal.svelte';
	import FixMatchModal from '$lib/components/FixMatchModal.svelte';
	import CastRow from '$lib/components/CastRow.svelte';
	import { fetchItemPeople, type Credit } from '$lib/media/people';
	import Icon from '$lib/components/Icon.svelte';
	import { ApiError } from '$lib/playback/api';
	import { downloadTicket, fetchMyAccess, startDownload } from '$lib/media/download';
	import type { IdentifyResult } from '$lib/media/identify';
	import ScenesRow from '$lib/components/ScenesRow.svelte';
	import StreamPicker from '$lib/components/StreamPicker.svelte';
	import SeasonGrid from '$lib/components/SeasonGrid.svelte';
	import { fetchSeasonCards, type SeasonCard } from '$lib/media/series';
	import MoreMenu from '$lib/components/MoreMenu.svelte';
	import {
		fetchItemDetails,
		formatRating,
		formatRuntime,
		preferredSubtitle,
		watchQuery,
		type ItemDetails
	} from '$lib/media/details';
	import { endsAt, formatClock } from '$lib/player/format';
	import { loadPreferences } from '$lib/player/preferences';
	import type { MediaState } from '$lib/playback/api';

	const itemId = $derived(page.params.id ?? '');

	let item = $state<MediaItem | null>(null);
	let token = $state('');
	let loading = $state(true);
	let notFound = $state(false);
	let loadFailed = $state(false);
	let backdropFailed = $state(false);
	let logoFailed = $state(false);
	let posterFailed = $state(false);
	let providers: ProviderConfig[] = $state([]);
	let editing = $state(false);
	/** Only ever populated for a `'series'` item — see the "Sezony" fetch
	 * in `load()` below. Stays `[]` for a movie. */
	let seasons: SeriesSeason[] = $state([]);
	/** Faza 6: the collection a movie belongs to ("Z kolekcji" row). */
	let collection = $state<Collection | null>(null);
	/** Faza 8: cast and crew ("Obsada i ekipa" row). */
	let credits = $state<Credit[]>([]);
	/** Faza 10: the season cards of a series. */
	let seasonCards = $state<SeasonCard[]>([]);
	/** Faza 9: tagline, genres, crew, streams, chapters. */
	let details = $state<ItemDetails | null>(null);
	/** Streams picked for "Play" (`null` audio = the server's default). */
	let audioChoice = $state<number | null>(null);
	let subtitleChoice = $state('');
	/** Watched / favourite / resume point (from MediaActions). */
	let mediaState = $state<MediaState | null>(null);
	/** Wall clock for "Koniec o ..." (moves on while the page is open). */
	let now = $state(Date.now());
	/** Faza 7: "Napraw dopasowanie" (admin) and downloads. */
	let fixing = $state(false);
	let fixedTitle = $state<string | null>(null);
	let canDownload = $state(false);
	let downloading = $state<string | null>(null);
	let downloadErrorKey = $state<string | null>(null);
	/** Changes after a fix so the browser loads the new artwork, not its
	 * hour-long cached copy of the old one. */
	let imageVersion = $state(0);
	const img = (path: string) =>
		imageUrl(path, token) + (imageVersion ? `&v=${imageVersion}` : '');

	async function download(id: string) {
		if (!token || downloading) return;
		downloading = id;
		downloadErrorKey = null;
		try {
			startDownload(await downloadTicket(token, id));
		} catch (err) {
			downloadErrorKey =
				err instanceof ApiError && err.code === 'downloads_disabled'
					? 'download.disabled'
					: 'download.failed';
		} finally {
			downloading = null;
		}
	}

	function identified(result: IdentifyResult) {
		fixing = false;
		fixedTitle = result.title;
		imageVersion = Date.now();
		triggerLoad(itemId);
	}

	/** Bumped once per load attempt; guards every state write below (the
	 * primary fetch and both non-fatal secondary ones) against an older
	 * navigation's response landing after a newer one already has — see
	 * this file's own top comment. */
	let loadSeq = 0;

	async function loadForId(id: string, mySeq: number) {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;

		let loadedItem: MediaItem;
		try {
			loadedItem = await fetchMediaDetail(sessionToken, id);
		} catch (err) {
			if (mySeq !== loadSeq) return; // superseded — discard
			if (err instanceof MediaNotFoundError) {
				notFound = true;
			} else {
				loadFailed = true;
			}
			loading = false;
			return;
		}
		if (mySeq !== loadSeq) return; // superseded — discard
		item = loadedItem;
		loading = false;

		fetchItemDetails(sessionToken, loadedItem.id)
			.then((loaded) => {
				if (mySeq !== loadSeq) return;
				details = loaded;
				audioChoice = loaded.default_audio_index ?? null;
				subtitleChoice = preferredSubtitle(
					loaded.subtitle_tracks,
					loadPreferences().subtitleLanguage
				);
			})
			.catch(() => {
				// The page works without the extras.
			});

		fetchItemPeople(sessionToken, loadedItem.id)
			.then((loaded) => {
				if (mySeq === loadSeq) credits = loaded;
			})
			.catch(() => {
				// Decoration only: the page works without the row.
			});

		fetchMyAccess(sessionToken)
			.then((access) => {
				if (mySeq === loadSeq) canDownload = access.can_download;
			})
			.catch(() => {
				// No button rather than a broken one.
			});

		if (page.data.user?.is_admin) {
			try {
				const loadedProviders = await fetchProviders(sessionToken);
				if (mySeq === loadSeq) providers = loadedProviders;
			} catch {
				// Non-fatal: "Edytuj metadane" would just open with an empty
				// forced-provider dropdown (every other field still works)
				// rather than failing the whole Detail View over this
				// secondary, admin-only fetch.
			}
		}

		if (loadedItem.item_type === 'movie') {
			try {
				const loadedCollection = await fetchItemCollection(sessionToken, loadedItem.id);
				if (mySeq === loadSeq) collection = loadedCollection;
			} catch {
				// Decoration only: the page works without the row.
			}
		}

		if (loadedItem.item_type === 'series') {
			void reloadSeasonCards(loadedItem.id, mySeq);
			try {
				const loadedSeasons = await fetchSeriesSeasons(sessionToken, loadedItem.id);
				if (mySeq === loadSeq) {
					seasons = loadedSeasons;
				}
			} catch {
				// Non-fatal, same reasoning as the provider fetch above: the
				// rest of the page (backdrop, title, overview, Play/Edytuj)
				// is already fully usable without the "Sezony" section —
				// losing just that one section over a secondary fetch is
				// better than failing the whole Detail View.
			}
		}
	}

	async function reloadSeasonCards(id: string, mySeq = loadSeq) {
		if (!token || !id) return;
		try {
			const cards = await fetchSeasonCards(token, id);
			if (mySeq === loadSeq) seasonCards = cards;
		} catch {
			// The seasons strip of an older server is not worth an error.
		}
	}

	function triggerLoad(id: string) {
		const mySeq = ++loadSeq;
		// Reset SYNCHRONOUSLY, before the async fetch starts, so the
		// previous item's content is never visible under the new URL, even
		// for one frame.
		loading = true;
		notFound = false;
		loadFailed = false;
		backdropFailed = false;
		logoFailed = false;
		posterFailed = false;
		item = null;
		seasons = [];
		collection = null;
		credits = [];
		seasonCards = [];
		details = null;
		audioChoice = null;
		subtitleChoice = '';
		mediaState = null;
		loadForId(id, mySeq);
	}

	$effect(() => {
		const id = itemId; // read synchronously — this is what Svelte tracks
		triggerLoad(id);
	});

	/** A movie shows its year; a series shows nothing here (there's no
	 * single year for a whole show — see `lisek_db`'s `home_candidates`
	 * CTE doc comment) — the "Sezony" section below carries a show's own
	 * context instead. */
	let subtitle = $derived.by(() => {
		if (!item) return '';
		return item.year ? String(item.year) : '';
	});

	let age = $derived(item ? ageLabel(item) : null);

	$effect(() => {
		const timer = setInterval(() => (now = Date.now()), 15_000);
		return () => clearInterval(timer);
	});

	const isMovie = $derived(item?.item_type === 'movie');
	/** What "Play" opens: a film itself; for a series the first episode not
	 * watched yet (regular seasons before specials), else the first one. */
	const playId = $derived.by(() => {
		if (!item) return '';
		if (item.item_type !== 'series') return item.id;
		const next = seasonCards.find((c) => c.unwatched_count > 0) ?? seasonCards[0];
		return next?.play_id ?? seasons[0]?.episodes[0]?.id ?? item.id;
	});
	const started = $derived(
		!!mediaState && !mediaState.watched && mediaState.position_seconds > 5
	);
	/** The player for this page's choices, from the resume point or `start`. */
	function watchHref(start: number | null = null): string {
		const query = isMovie
			? watchQuery({ audio: audioChoice, subtitle: subtitleChoice, start })
			: watchQuery({ start });
		return resolve('/watch/[id]', { id: playId }) + query;
	}
	const runtime = $derived(formatRuntime(details?.runtime_minutes));
	const rating = $derived(formatRating(details?.rating));
	/** "Koniec o 12:50": now plus what is left (from the resume point). */
	const endTime = $derived.by(() => {
		if (!isMovie || !details?.runtime_minutes) return null;
		const resumeAt = started && mediaState ? mediaState.position_seconds : 0;
		const end = endsAt(now, Math.max(0, details.runtime_minutes * 60 - resumeAt));
		return end ? formatClock(end, getLocale()) : null;
	});
	const moreEntries = $derived([
		...(canDownload && isMovie && item
			? [
					{
						labelKey: downloading ? 'download.preparing' : 'download.button',
						disabled: downloading !== null,
						onselect: () => item && download(item.id)
					}
				]
			: []),
		...(page.data.user?.is_admin
			? [
					{ labelKey: 'admin.editor_title', onselect: () => (editing = true) },
					{ labelKey: 'fix_match.button', onselect: () => (fixing = true) }
				]
			: [])
	]);
	const actionButton =
		'grid size-11 place-items-center rounded-full text-neutral-100 transition-colors hover:bg-white/10 hover:text-gold focus-visible:bg-white/15 focus-visible:text-gold focus-visible:outline-2 focus-visible:outline-gold';

</script>

<main class="min-h-screen bg-base pb-16">
	{#if loading}
		<div class="flex min-h-screen items-center justify-center">
			<p class="text-sm text-neutral-400">{t('common.loading')}</p>
		</div>
	{:else if notFound}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-neutral-400">{t('detail.not_found')}</p>
			<a
				href={resolve('/')}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('detail.back_to_home')}
			</a>
		</div>
	{:else if loadFailed}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-red-400">{t('detail.load_failed')}</p>
			<button
				type="button"
				onclick={() => triggerLoad(itemId)}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('home.retry')}
			</button>
		</div>
	{:else if item}
		<!--
		  Faza 9: the reference layout. A backdrop band with the title logo,
		  the poster overlapping it on the left; on the right a title strip
		  (title, original title, year · runtime · age · rating · end time,
		  round action buttons), the streams to play with, tagline and
		  overview, then tags, links, genres, crew and studios.
		-->
		<section class="relative h-[240px] w-full overflow-hidden bg-surface sm:h-[300px] md:h-[360px]">
			{#if !backdropFailed}
				<img
					src={img(item.backdrop_url)}
					alt=""
					onerror={() => (backdropFailed = true)}
					class="absolute inset-0 size-full object-cover object-[center_25%] opacity-40"
				/>
			{:else}
				<div class="absolute inset-0 bg-gradient-to-br from-surface via-black to-surface"></div>
			{/if}
			<div class="absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-black/80 to-transparent"></div>
			<div class="absolute inset-0 bg-gradient-to-t from-base via-base/30 to-transparent"></div>
			{#if item.logo_url && !logoFailed}
				<div
					class="absolute inset-x-4 top-20 bottom-6 flex items-center justify-center md:inset-x-auto md:right-[8%] md:left-[38%]"
				>
					<img
						src={img(item.logo_url)}
						alt=""
						onerror={() => (logoFailed = true)}
						class="max-h-full max-w-full object-contain drop-shadow-[0_4px_12px_rgb(0_0_0/0.7)]"
					/>
				</div>
			{/if}
		</section>

		<div class="relative mx-auto max-w-[1600px] px-4 md:px-8 lg:px-12">
			<div class="flex flex-col gap-6 md:flex-row md:gap-10">
				<div
					class="mx-auto -mt-28 w-40 shrink-0 sm:w-48 md:mx-0 md:-mt-48 md:w-64 lg:w-[300px]"
				>
					<div
						class="aspect-[2/3] overflow-hidden rounded-lg bg-surface shadow-[0_20px_50px_rgb(0_0_0/0.7)] ring-1 ring-white/10"
					>
						{#if !posterFailed}
							<img
								src={img(item.poster_url)}
								alt=""
								onerror={() => (posterFailed = true)}
								class="size-full object-cover"
							/>
						{/if}
					</div>
				</div>

				<div class="min-w-0 flex-1">
					<div
						class="flex flex-col gap-4 border-b border-white/10 pb-5 md:-mt-2 lg:flex-row lg:items-center lg:justify-between"
					>
						<div class="min-w-0 text-center md:text-left">
							<h1 class="text-2xl leading-tight font-semibold text-white md:text-4xl">
								{item.title}
							</h1>
							{#if details?.original_title}
								<p class="mt-1 text-sm font-semibold text-neutral-300 md:text-[1rem]">
									{details.original_title}
								</p>
							{/if}
							<div
								class="mt-3 flex flex-wrap items-center justify-center gap-2 text-sm text-neutral-200 md:justify-start"
							>
								{#if subtitle}<span class="chip">{subtitle}</span>{/if}
								{#if runtime && isMovie}<span class="chip">{runtime}</span>{/if}
								{#if age}
									<span
										class="rounded border border-gold/70 px-2 py-0.5 text-xs font-semibold text-gold"
										title={t('detail.age_rating_hint')}>{age}</span
									>
								{/if}
								{#if rating}
									<span class="chip flex items-center gap-1" title={t('details.rating_hint')}>
										<Icon name="star" class="size-4 text-gold" />
										{rating}
									</span>
								{/if}
								{#if endTime}
									<span class="chip" data-testid="detail-ends-at"
										>{t('details.ends_at', { time: endTime })}</span
									>
								{/if}
							</div>
						</div>

						<div class="flex items-center justify-center gap-1 md:justify-start lg:justify-end">
							<!-- eslint-disable svelte/no-navigation-without-resolve -- watchHref() builds on resolve() -->
							<a
								href={watchHref()}
								class="{actionButton} bg-gold text-black hover:bg-gold-dim hover:text-black focus-visible:bg-gold-dim focus-visible:text-black"
								aria-label={started ? t('details.resume') : t('home.play')}
								title={started ? t('details.resume') : t('home.play')}
								data-nav-default
							>
								<Icon name="play" class="size-6" />
							</a>
							{#if started}
								<a
									href={watchHref(0)}
									class={actionButton}
									aria-label={t('details.start_over')}
									title={t('details.start_over')}
								>
									<Icon name="replay" class="size-6" />
								</a>
							{/if}
							<!-- eslint-enable svelte/no-navigation-without-resolve -->
							<MediaActions itemId={item.id} {token} onstate={(s) => (mediaState = s)} />
							<MoreMenu entries={moreEntries} />
						</div>
					</div>

					{#if downloadErrorKey}
						<p role="alert" class="mt-3 text-sm text-red-300">{t(downloadErrorKey)}</p>
					{/if}
					{#if fixedTitle}
						<p role="status" class="mt-3 text-sm text-gold">
							{t('fix_match.done', { title: fixedTitle })}
						</p>
					{/if}

					{#if isMovie && details}
						<div class="mt-5">
							<StreamPicker {details} bind:audio={audioChoice} bind:subtitle={subtitleChoice} />
						</div>
					{/if}

					<div class="mt-6 max-w-5xl">
						{#if details?.tagline}
							<p class="mb-3 font-semibold text-white md:text-lg">{details.tagline}</p>
						{/if}
						{#if item.overview}
							<p class="text-sm leading-relaxed text-neutral-300 md:text-[0.95rem] md:leading-7">
								{item.overview}
							</p>
						{/if}
					</div>

					{#if details}
						<div class="mt-6 flex max-w-5xl flex-col gap-3 text-sm">
							{#if details.tags.length > 0}
								<p class="leading-7">
									<span class="mr-1 text-neutral-200">{t('details.tags')}</span>
									<span class="text-gold">{details.tags.join(', ')}</span>
								</p>
							{/if}
							{#if details.external_links.length > 0}
								<p class="commas">
									<!-- eslint-disable svelte/no-navigation-without-resolve -- external sites -->
									{#each details.external_links as link (link.url)}
										<a
											href={link.url}
											target="_blank"
											rel="noopener noreferrer"
											class="text-gold hover:underline focus-visible:outline-2">{link.name}</a
										>
									{/each}
									<!-- eslint-enable svelte/no-navigation-without-resolve -->
								</p>
							{/if}
						</div>

						<dl class="mt-5 grid max-w-5xl grid-cols-[6.5rem_1fr] gap-x-4 gap-y-2 text-sm">
							{#if details.genres.length > 0}
								<dt class="text-neutral-300">{t('details.genres')}</dt>
								<dd class="text-gold">{details.genres.join(', ')}</dd>
							{/if}
							{#if details.directors.length > 0}
								<dt class="text-neutral-300">{t('details.director')}</dt>
								<dd class="commas">
									{#each details.directors as person (person.person_id)}
										<a
											href={resolve('/person/[id]', { id: person.person_id })}
											class="text-gold hover:underline focus-visible:outline-2">{person.name}</a
										>
									{/each}
								</dd>
							{/if}
							{#if details.writers.length > 0}
								<dt class="text-neutral-300">{t('details.writers')}</dt>
								<dd class="commas">
									{#each details.writers as person (person.person_id)}
										<a
											href={resolve('/person/[id]', { id: person.person_id })}
											class="text-gold hover:underline focus-visible:outline-2">{person.name}</a
										>
									{/each}
								</dd>
							{/if}
							{#if details.studios.length > 0}
								<dt class="text-neutral-300">{t('details.studios')}</dt>
								<dd class="text-gold">{details.studios.join(', ')}</dd>
							{/if}
						</dl>
					{/if}
				</div>
			</div>

			{#if item.item_type === 'series' && seasonCards.length > 0}
				<div class="mt-10 border-t border-white/10 pt-8">
					<SeasonGrid
						seriesId={item.id}
						bind:seasons={seasonCards}
						{token}
						onchange={() => reloadSeasonCards(item?.id ?? '')}
					/>
				</div>
			{/if}
		</div>

		<div class="mt-10">
			<CastRow {credits} {token} />
			{#if details}
				<ScenesRow chapters={details.chapters} {token} hrefFor={(seconds) => watchHref(seconds)} />
			{/if}
		</div>

		{#if collection && collection.items.length > 1}
			<section class="mt-2" aria-label={collection.name}>
				<div class="mb-2 flex items-baseline justify-between gap-4 px-4 md:px-8">
					<h2 class="text-lg font-semibold text-neutral-100">
						{t('collections.from_collection', { name: collection.name })}
					</h2>
					<a
						href={resolve('/collection/[id]', { id: collection.id })}
						class="shrink-0 text-sm text-gold hover:underline focus-visible:outline-2"
					>
						{t('collections.see_collection')}
					</a>
				</div>
				<Carousel>
					{#each collection.items as film (film.id)}
						<MediaCard item={film} {token} />
					{/each}
				</Carousel>
			</section>
		{/if}
	{/if}
</main>

{#if editing && item}
	<MediaItemEditorModal
		itemId={item.id}
		{providers}
		onClose={() => (editing = false)}
		onSaved={() => triggerLoad(itemId)}
	/>
{/if}

{#if fixing && item}
	<FixMatchModal
		itemId={item.id}
		kind={item.item_type === 'movie' ? 'movie' : 'series'}
		title={item.title}
		year={item.year}
		onClose={() => (fixing = false)}
		onIdentified={identified}
	/>
{/if}

<style>
	.chip {
		border-radius: 0.3rem;
		background: rgb(255 255 255 / 0.1);
		padding: 0.15rem 0.6rem;
	}
	/* "IMDb, TheTVDB, TMDB": commas drawn after every link but the last. */
	.commas > :global(a:not(:last-of-type))::after {
		content: ',';
		margin-right: 0.3rem;
		color: #737373;
	}
</style>
