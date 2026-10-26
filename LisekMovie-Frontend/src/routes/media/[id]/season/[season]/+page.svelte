<!--
  Faza 10 — one season of a series: the series backdrop with the season
  poster, the season's name, year and synopsis, Play (the first episode
  not watched yet, from its resume point), watched / favourite for the
  whole season, then every episode as a vertical list (`EpisodeList`).
  "Informacje" on an episode opens `EpisodeInfoModal` with the stream
  picks of a film page. Same `$effect` + `loadSeq` pattern as
  `/media/[id]`: moving between seasons never shows the previous one.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchMediaDetail, imageUrl, MediaNotFoundError, type MediaItem } from '$lib/media/api';
	import { downloadTicket, fetchMyAccess, startDownload } from '$lib/media/download';
	import {
		fetchSeasonPage,
		seasonTitle,
		setSeasonState,
		type EpisodeRow,
		type SeasonPage
	} from '$lib/media/series';
	import { ApiError } from '$lib/playback/api';
	import EpisodeInfoModal from '$lib/components/EpisodeInfoModal.svelte';
	import EpisodeList from '$lib/components/EpisodeList.svelte';
	import Icon from '$lib/components/Icon.svelte';
	import PlaystateButton from '$lib/components/PlaystateButton.svelte';
	import { PLAY_TONE, TOGGLE_SHAPE, toggleTone } from '$lib/components/toggleTone';

	const seriesId = $derived(page.params.id ?? '');
	const seasonNumber = $derived(Number(page.params.season ?? 'NaN'));

	let series = $state<MediaItem | null>(null);
	let season = $state<SeasonPage | null>(null);
	let token = $state('');
	let loading = $state(true);
	let notFound = $state(false);
	let loadFailed = $state(false);
	let backdropFailed = $state(false);
	let posterFailed = $state(false);
	let canDownload = $state(false);
	let info = $state<EpisodeRow | null>(null);
	let busy = $state(false);
	let actionError = $state<string | null>(null);
	let now = $state(Date.now());
	let loadSeq = 0;

	async function load(id: string, number: number, mySeq: number) {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;
		if (!Number.isInteger(number) || number < 0) {
			notFound = true;
			loading = false;
			return;
		}
		try {
			const [loadedSeries, loadedSeason] = await Promise.all([
				fetchMediaDetail(sessionToken, id),
				fetchSeasonPage(sessionToken, id, number)
			]);
			if (mySeq !== loadSeq) return;
			series = loadedSeries;
			season = loadedSeason;
		} catch (err) {
			if (mySeq !== loadSeq) return;
			if (err instanceof MediaNotFoundError || (err instanceof ApiError && err.status === 404))
				notFound = true;
			else loadFailed = true;
		} finally {
			if (mySeq === loadSeq) loading = false;
		}
		fetchMyAccess(sessionToken)
			.then((access) => {
				if (mySeq === loadSeq) canDownload = access.can_download;
			})
			.catch(() => {
				// No download entries rather than broken ones.
			});
	}

	function triggerLoad(id: string, number: number) {
		const mySeq = ++loadSeq;
		loading = true;
		notFound = false;
		loadFailed = false;
		backdropFailed = false;
		posterFailed = false;
		series = null;
		season = null;
		info = null;
		void load(id, number, mySeq);
	}

	$effect(() => {
		const id = seriesId;
		const number = seasonNumber;
		triggerLoad(id, number);
	});

	$effect(() => {
		const timer = setInterval(() => (now = Date.now()), 15_000);
		return () => clearInterval(timer);
	});

	const title = $derived(season ? seasonTitle(season, t) : '');
	const unwatched = $derived(season?.episodes.filter((e) => !e.watched).length ?? 0);
	const allFavorite = $derived(!!season && season.episodes.every((e) => e.favorite));
	/** Play: the first episode not watched yet, else the first. */
	const playId = $derived(
		season ? (season.episodes.find((e) => !e.watched) ?? season.episodes[0])?.id : undefined
	);

	async function toggleSeason(key: 'watched' | 'favorite') {
		if (!season || busy) return;
		busy = true;
		actionError = null;
		const value = key === 'watched' ? unwatched > 0 : !allFavorite;
		try {
			await setSeasonState(token, seriesId, season.season, { [key]: value });
			season = {
				...season,
				episodes: season.episodes.map((e) =>
					key === 'watched'
						? { ...e, watched: value, position_seconds: 0 }
						: { ...e, favorite: value }
				)
			};
		} catch {
			actionError = 'player.save_failed';
		} finally {
			busy = false;
		}
	}

	async function download(episode: EpisodeRow) {
		actionError = null;
		try {
			startDownload(await downloadTicket(token, episode.id));
		} catch (err) {
			actionError =
				err instanceof ApiError && err.code === 'downloads_disabled'
					? 'download.disabled'
					: 'download.failed';
		}
	}

	const round = `${TOGGLE_SHAPE} size-11`;
</script>

<svelte:head>
	<title>{series && season ? `${series.title} · ${title} · LisekMovie` : 'LisekMovie'}</title>
</svelte:head>

<main class="min-h-screen bg-base pb-16">
	{#if loading}
		<div class="flex min-h-screen items-center justify-center">
			<p class="text-sm text-neutral-400">{t('common.loading')}</p>
		</div>
	{:else if notFound}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-neutral-400">{t('series.season_not_found')}</p>
			<a
				href={resolve('/media/[id]', { id: seriesId })}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('series.back_to_series')}
			</a>
		</div>
	{:else if loadFailed}
		<div class="flex min-h-screen flex-col items-center justify-center gap-4 px-4 text-center">
			<p class="text-sm text-red-400">{t('detail.load_failed')}</p>
			<button
				type="button"
				onclick={() => triggerLoad(seriesId, seasonNumber)}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('home.retry')}
			</button>
		</div>
	{:else if series && season}
		<section class="relative h-[200px] w-full overflow-hidden bg-surface sm:h-[260px] md:h-[300px]">
			{#if !backdropFailed}
				<img
					src={imageUrl(series.backdrop_url, token)}
					alt=""
					onerror={() => (backdropFailed = true)}
					class="absolute inset-0 size-full object-cover object-[center_25%] opacity-40"
				/>
			{/if}
			<div
				class="absolute inset-x-0 top-0 h-28 bg-gradient-to-b from-black/80 to-transparent"
			></div>
			<div class="absolute inset-0 bg-gradient-to-t from-base via-base/30 to-transparent"></div>
		</section>

		<div class="relative mx-auto max-w-[1600px] px-4 md:px-8 lg:px-12">
			<div class="flex flex-col gap-6 md:flex-row md:gap-10">
				<div class="mx-auto -mt-24 w-36 shrink-0 sm:w-44 md:mx-0 md:-mt-40 md:w-56 lg:w-64">
					<div
						class="aspect-[2/3] overflow-hidden rounded-lg bg-surface shadow-[0_20px_50px_rgb(0_0_0/0.7)] ring-1 ring-white/10"
					>
						{#if !posterFailed}
							<img
								src={imageUrl(season.poster_url, token)}
								alt=""
								onerror={() => (posterFailed = true)}
								class="size-full object-cover"
							/>
						{/if}
					</div>
				</div>

				<div class="min-w-0 flex-1 text-center md:text-left">
					<a
						href={resolve('/media/[id]', { id: seriesId })}
						class="inline-flex items-center gap-1 text-sm font-medium text-gold hover:underline focus-visible:outline-2"
					>
						<Icon name="chevron_left" class="size-4" />
						{series.title}
					</a>
					<h1 class="mt-1 text-2xl leading-tight font-semibold text-white md:text-4xl">{title}</h1>
					<div
						class="mt-3 flex flex-wrap items-center justify-center gap-2 text-sm text-neutral-200 md:justify-start"
					>
						{#if season.air_year}<span class="chip">{season.air_year}</span>{/if}
						<span class="chip">{t('series.episode_count', { count: season.episodes.length })}</span>
						{#if unwatched > 0 && unwatched < season.episodes.length}
							<span class="chip">{t('series.unwatched_count', { count: unwatched })}</span>
						{/if}
					</div>
					<div class="mt-4 flex items-center justify-center gap-1 md:justify-start">
						{#if playId}
							<a
								href={resolve('/watch/[id]', { id: playId })}
								data-nav-default
								class="{round} {PLAY_TONE}"
								aria-label={t('home.play')}
								title={t('home.play')}
							>
								<Icon name="play" class="size-6" />
							</a>
						{/if}
						<PlaystateButton
							played={unwatched === 0}
							disabled={busy}
							onclick={() => toggleSeason('watched')}
						/>
						<button
							type="button"
							class="{round} {toggleTone('favorite', allFavorite)}"
							disabled={busy}
							aria-pressed={allFavorite}
							aria-label={t(allFavorite ? 'player.unfavorite' : 'player.favorite')}
							title={t(allFavorite ? 'player.unfavorite' : 'player.favorite')}
							onclick={() => toggleSeason('favorite')}
						>
							<Icon name={allFavorite ? 'heart_filled' : 'heart'} class="size-6" />
						</button>
					</div>
					{#if actionError}
						<p role="alert" class="mt-2 text-sm text-gold">{t(actionError)}</p>
					{/if}
					{#if season.overview}
						<p
							class="mt-4 max-w-4xl text-sm leading-relaxed text-neutral-300 md:text-[0.95rem] md:leading-7"
						>
							{season.overview}
						</p>
					{/if}
				</div>
			</div>

			<section class="mt-8 md:mt-10" aria-labelledby="episodes-title">
				<h2 id="episodes-title" class="mb-3 text-lg font-semibold text-neutral-100 md:text-xl">
					{t('series.episodes')}
				</h2>
				<EpisodeList
					bind:episodes={season.episodes}
					{token}
					{now}
					{canDownload}
					oninfo={(episode) => (info = episode)}
					ondownload={download}
				/>
			</section>
		</div>
	{/if}
</main>

{#if info && season}
	<EpisodeInfoModal
		episode={info}
		context={series ? `${series.title} · ${title}` : title}
		{token}
		{now}
		onclose={() => (info = null)}
	/>
{/if}

<style>
	.chip {
		border-radius: 0.3rem;
		background: rgb(255 255 255 / 0.1);
		padding: 0.15rem 0.6rem;
	}
</style>
