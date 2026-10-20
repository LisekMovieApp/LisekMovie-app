<!--
  My List: continue watching, favourites and watched, from `/api/me/list`
  (newest activity first). Same cards as the home rows; "continue" cards
  open the player directly and show the progress the telemetry saved.
  The grid is one spatial-navigation group, so a remote returns to the
  card it left.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchMyList, type ListEntry } from '$lib/playback/api';
	import { entryAsItem, inProgress, minutesLeft, progressOf } from '$lib/media/list';
	import { navGroup } from '$lib/spatial/spatial';
	import MediaCard from '$lib/components/MediaCard.svelte';

	type Filter = 'continue' | 'favorites' | 'watched';
	const FILTERS: Filter[] = ['continue', 'favorites', 'watched'];

	let items = $state<ListEntry[]>([]);
	let token = $state('');
	let loading = $state(true);
	let failed = $state(false);
	let filter = $state<Filter>('continue');

	const visible = $derived(
		items.filter((item) =>
			filter === 'favorites'
				? item.favorite
				: filter === 'watched'
					? item.watched
					: inProgress(item)
		)
	);

	function caption(entry: ListEntry): string | null {
		const episode =
			entry.season !== null && entry.episode !== null
				? t('home.episode_short', { season: entry.season, episode: entry.episode })
				: null;
		const left = inProgress(entry) ? t('home.minutes_left', { minutes: minutesLeft(entry) }) : null;
		return [episode, left].filter(Boolean).join(' · ') || null;
	}

	async function load() {
		token = getSessionCookieClientSide() ?? '';
		loading = true;
		failed = false;
		try {
			items = await fetchMyList(token);
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	onMount(load);
</script>

<main class="min-h-screen bg-base px-4 pt-24 pb-16 text-white md:px-8 md:pt-28">
	<h1 class="mb-6 text-3xl font-bold tracking-tight md:text-4xl">{t('nav.my_list')}</h1>
	<div class="mb-8 flex flex-wrap gap-2" role="group" aria-label={t('nav.my_list')}>
		{#each FILTERS as key (key)}
			<button
				type="button"
				class="rounded-full px-5 py-2 text-sm font-semibold transition-colors {filter === key
					? 'bg-white text-black'
					: 'bg-white/10 text-neutral-200 hover:bg-white/20'}"
				aria-pressed={filter === key}
				onclick={() => (filter = key)}>{t(`player.list_${key}`)}</button
			>
		{/each}
	</div>
	{#if loading}
		<div
			class="grid grid-cols-2 gap-x-3 gap-y-6 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5"
			aria-hidden="true"
		>
			{#each { length: 10 } as _, i (i)}
				<div class="aspect-video animate-pulse rounded-md bg-surface"></div>
			{/each}
		</div>
		<p class="sr-only" role="status">{t('common.loading')}</p>
	{:else if failed}
		<div class="flex flex-col items-start gap-3" role="alert">
			<p class="text-neutral-300">{t('player.load_failed')}</p>
			<button
				type="button"
				class="rounded-md bg-white/10 px-4 py-2 hover:bg-white/20"
				onclick={load}>{t('home.retry')}</button
			>
		</div>
	{:else if visible.length === 0}
		<p class="text-neutral-400">{t('player.list_empty')}</p>
	{:else}
		<div
			class="grid grid-cols-2 gap-x-3 gap-y-6 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5 [&>a]:w-full"
			use:navGroup
		>
			{#each visible as item (item.id)}
				<MediaCard
					item={entryAsItem(item)}
					{token}
					href={filter === 'continue'
						? resolve('/watch/[id]', { id: item.id })
						: resolve('/media/[id]', { id: item.id })}
					caption={caption(item)}
					progress={filter === 'continue' ? progressOf(item) : null}
				/>
			{/each}
		</div>
	{/if}
</main>
