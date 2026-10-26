<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
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
	/** Which season's episodes the picker strip below is currently
	 * showing — defaults to the first season once `seasons` loads (set at
	 * the end of `load()`), so the section never opens empty. */
	let selectedSeason = $state<number | null>(null);
	/** Faza 6: the collection a movie belongs to ("Z kolekcji" row). */
	let collection = $state<Collection | null>(null);
	/** Faza 8: cast and crew ("Obsada i ekipa" row). */
	let credits = $state<Credit[]>([]);
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
			try {
				const loadedSeasons = await fetchSeriesSeasons(sessionToken, loadedItem.id);
				if (mySeq === loadSeq) {
					seasons = loadedSeasons;
					selectedSeason = loadedSeasons[0]?.season ?? null;
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
		selectedSeason = null;
		collection = null;
		credits = [];
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

	/** The episode list the season-picker strip shows below itself —
	 * whichever season in `seasons` matches `selectedSeason`. */
	let age = $derived(item ? ageLabel(item) : null);

	let activeSeasonEpisodes = $derived.by(() => {
		return seasons.find((s) => s.season === selectedSeason)?.episodes ?? [];
	});
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
		<section
			class="relative h-[70vh] max-h-[680px] min-h-[380px] w-full overflow-hidden bg-surface"
		>
			{#if !backdropFailed}
				<img
					src={img(item.backdrop_url)}
					alt=""
					onerror={() => (backdropFailed = true)}
					class="absolute inset-0 size-full object-cover"
				/>
			{:else}
				<div class="absolute inset-0 bg-gradient-to-br from-surface via-black to-surface"></div>
				<div
					class="absolute inset-0 bg-[radial-gradient(ellipse_at_30%_20%,rgba(189,162,77,0.14),transparent_60%)]"
				></div>
			{/if}

			<div
				class="absolute inset-x-0 top-0 h-32 bg-gradient-to-b from-black/70 to-transparent"
			></div>
			<div
				class="absolute inset-x-0 bottom-0 h-2/3 bg-gradient-to-t from-base via-base/70 to-transparent"
			></div>

			<!--
			  Was `top-4 md:top-8` — squarely underneath the app's own
			  `fixed top-0 z-50` navbar (`Navbar.svelte`), which made it
			  completely unclickable (reported after a live test). That
			  header is `py-3` (0.75rem top+bottom padding) around a `h-16`
			  logo (`h-13` once scrolled) — 88px tall unscrolled, 76px
			  scrolled, at every breakpoint alike (the logo has no
			  responsive height class, so neither does the header itself).
			  `top-24` (96px) is verified — not guessed — to clear both
			  states with margin to spare, on every screen size, so a
			  single value replaces the old two-tier `top-4 md:top-8`.
			-->
			<a
				href={resolve('/')}
				class="absolute top-24 left-4 z-10 rounded-md bg-black/40 px-3 py-1.5 text-sm text-neutral-100 backdrop-blur-sm transition-colors hover:bg-black/60 md:left-8"
			>
				{t('detail.back_to_home')}
			</a>

			<div class="absolute bottom-0 left-0 flex w-full max-w-2xl flex-col gap-4 p-4 md:p-8 lg:p-12">
				<!--
				  Both branches share one min-height wrapper (`items-end`) so
				  swapping between them — a 404'd logo falling back to text —
				  never visibly jumps the row below it. Without a shared
				  reserved height, the two branches' naturally different
				  heights made that swap read as "the layout just broke" for
				  the instant it happened, exactly as reported after a live
				  test against a title with no downloaded logo yet.
				-->
				<div class="flex min-h-24 items-end md:min-h-36">
					{#if item.logo_url && !logoFailed}
						<img
							src={img(item.logo_url)}
							alt={item.title}
							onerror={() => (logoFailed = true)}
							class="max-h-24 max-w-full object-contain object-left drop-shadow-md md:max-h-36"
						/>
					{:else}
						<h1 class="text-3xl leading-tight font-bold text-white drop-shadow md:text-5xl">
							{item.title}
						</h1>
					{/if}
				</div>
			</div>
		</section>

		<!--
		  The Jellyfin-style "poster + metadata" row asked for in Runda 7.
		  Subtitle, the action buttons, and the overview all moved down here
		  from the hero overlay above — a more natural home for an action
		  row than floating over the backdrop, and it gives the repeated
		  text title somewhere to live for a title whose logo image is
		  small or stylized rather than fully legible on its own.

		  Deliberately plain document flow, with no negative margin pulling
		  the poster up to overlap the backdrop the way Jellyfin's own
		  layout does: the ask was also to avoid overlapping CSS/z-index
		  layers, and the bottom-left backdrop overlay above already
		  occupies that same corner (the title logo/text) — overlapping a
		  second element into it for a purely cosmetic gain would have
		  reintroduced exactly the risk that was asked to be avoided.
		-->
		<div class="mx-auto max-w-5xl px-4 py-8 md:px-8 md:py-10 lg:px-12">
			<div class="flex flex-col gap-6 sm:flex-row sm:gap-8">
				<div class="mx-auto w-40 shrink-0 sm:mx-0 sm:w-48 md:w-56">
					<div class="aspect-[2/3] overflow-hidden rounded-md bg-surface">
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

				<div class="flex flex-1 flex-col gap-4 text-center sm:text-left">
					<div>
						<h2 class="text-xl font-semibold text-white md:text-2xl">{item.title}</h2>
						{#if subtitle || age}
							<p
								class="mt-1 flex items-center justify-center gap-2 text-sm text-neutral-400 sm:justify-start"
							>
								{#if subtitle}<span>{subtitle}</span>{/if}
								{#if age}
									<span
										class="rounded border border-neutral-500 px-1.5 py-px text-xs font-semibold text-neutral-200"
										title={t('detail.age_rating_hint')}>{age}</span
									>
								{/if}
							</p>
						{/if}
					</div>

					<div class="flex justify-center gap-3 sm:justify-start">
						<a
							href={resolve('/watch/[id]', { id: item.item_type === 'series' ? (seasons[0]?.episodes[0]?.id ?? item.id) : item.id })}
							class="rounded-md bg-gold px-6 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
						>
							{t('home.play')}
						</a>
						{#if canDownload && item.item_type === 'movie'}
							<button
								type="button"
								disabled={downloading !== null}
								onclick={() => item && download(item.id)}
								class="flex items-center gap-2 rounded-md bg-white/20 px-5 py-2.5 font-medium text-white backdrop-blur-sm transition-colors hover:bg-white/30 disabled:opacity-50"
							>
								<Icon name="download" class="size-5" />
								{downloading ? t('download.preparing') : t('download.button')}
							</button>
						{/if}
						{#if page.data.user?.is_admin}
							<button
								type="button"
								onclick={() => (editing = true)}
								class="rounded-md bg-white/20 px-6 py-2.5 font-medium text-white backdrop-blur-sm transition-colors hover:bg-white/30"
							>
								{t('admin.editor_title')}
							</button>
							<button
								type="button"
								onclick={() => (fixing = true)}
								class="rounded-md bg-white/20 px-5 py-2.5 font-medium text-white backdrop-blur-sm transition-colors hover:bg-white/30"
							>
								{t('fix_match.button')}
							</button>
						{/if}
					</div>
					{#if downloadErrorKey}
						<p role="alert" class="text-sm text-red-300">{t(downloadErrorKey)}</p>
					{/if}
					{#if fixedTitle}
						<p role="status" class="text-sm text-gold">
							{t('fix_match.done', { title: fixedTitle })}
						</p>
					{/if}

					<MediaActions itemId={item.id} {token} />
					{#if item.overview}
						<!--
						  `md:text-[1rem] md:leading-6`, not `md:text-base` — the
						  exact same theme-key collision already documented (and
						  fixed) in `HeroBanner.svelte`'s own comment: this
						  project's `--color-base` token claims the "base" name
						  project-wide, so `md:text-base` compiled to ONLY a
						  color rule (near-black) at that breakpoint, with no
						  font-size rule — invisible on the same near-black
						  Detail View background. Root cause of the "black text
						  on black background on desktop" report: correct,
						  visible `text-neutral-200` on mobile (no `md:` override
						  in play there), silently overridden to near-black the
						  moment the `md:` breakpoint activated. This paragraph
						  moved from the hero overlay into this section in Runda
						  7 carrying the pre-existing class string as-is, without
						  re-applying the arbitrary-value fix already established
						  for this exact bug — the gap this round closes.
						-->
						<p class="text-sm leading-relaxed text-neutral-200 md:text-[1rem] md:leading-6">
							{item.overview}
						</p>
					{/if}
				</div>
			</div>

			{#if item.item_type === 'series' && seasons.length > 0}
				<!--
				  "Sezony": a season-picker strip (poster + "Sezon N", the show's
				  own poster reused for every one of them — no per-season artwork
				  exists, see `list_series_seasons`'s own doc comment) plus, below
				  it, the selected season's episode numbers. Deliberately plain
				  `<span>` pills, not links or buttons that look playable: there's
				  no per-episode page to send them to any more (an episode id now
				  always resolves back to this same series view — see
				  `lisek_db::get_media_item`'s doc comment) and no playback
				  pipeline either, so a control that looked actionable here would
				  be promising something this app can't yet do.
				-->
				<div class="mt-10 border-t border-white/10 pt-8">
					<h2 class="mb-4 text-lg font-semibold text-neutral-100 md:text-xl">
						{t('detail.seasons_title')}
					</h2>

					<div class="flex gap-3 overflow-x-auto pb-2">
						{#each seasons as s (s.season)}
							<button
								type="button"
								onclick={() => (selectedSeason = s.season)}
								class="w-28 shrink-0 rounded-md border-2 p-1 text-left transition-colors sm:w-32 {selectedSeason ===
								s.season
									? 'border-gold'
									: 'border-transparent hover:border-white/20'}"
							>
								<div class="aspect-[2/3] overflow-hidden rounded bg-surface">
									{#if !posterFailed}
										<img
											src={img(item.poster_url)}
											alt=""
											loading="lazy"
											class="size-full object-cover"
										/>
									{/if}
								</div>
								<p class="mt-1.5 truncate text-xs text-neutral-300">
									{t('detail.season_label', { number: s.season })}
								</p>
							</button>
						{/each}
					</div>

					{#if activeSeasonEpisodes.length > 0}
						<div class="mt-4 flex flex-wrap gap-2">
							{#each activeSeasonEpisodes as ep (ep.id)}
								<div class="flex overflow-hidden rounded-md bg-surface">
									<a href={resolve('/watch/[id]', { id: ep.id })} class="px-3 py-2 text-sm text-neutral-300 hover:text-gold focus-visible:outline-2">
										{ep.end_episode !== null
											? t('detail.episode_label_range', {
													number: ep.episode,
													end: ep.end_episode
												})
											: t('detail.episode_label', { number: ep.episode })}
									</a>
									{#if canDownload}
										<button
											type="button"
											disabled={downloading !== null}
											onclick={() => download(ep.id)}
											aria-label={t('download.episode', { number: ep.episode })}
											title={t('download.episode', { number: ep.episode })}
											class="border-l border-white/10 px-2 text-neutral-400 hover:text-gold focus-visible:outline-2 disabled:opacity-40"
										>
											<Icon name="download" class="size-4" />
										</button>
									{/if}
								</div>
							{/each}
						</div>
					{/if}
				</div>
			{/if}
		</div>

		<CastRow {credits} {token} />

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
