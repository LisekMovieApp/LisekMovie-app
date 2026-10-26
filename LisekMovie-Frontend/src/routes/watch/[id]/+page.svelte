<!--
  Watch page: the full-screen player. Details (title, episode, backdrop)
  load alongside the playback session only to label the player; when they
  fail the session's own title is enough. Leaving goes back to where the
  viewer came from inside the app, or to the details page.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { afterNavigate, goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchMediaDetail, imageUrl, type MediaItem } from '$lib/media/api';
	import Player from '$lib/player/Player.svelte';
	import { parseWatchQuery } from '$lib/media/details';

	const id = $derived(page.params.id ?? '');
	/** `?audio=&sub=&t=` from the detail page (streams, a scene). */
	const picked = $derived(parseWatchQuery(page.url.searchParams));
	let token = $state<string | null>(null);
	let item = $state<MediaItem | null>(null);
	let cameFromApp = false;

	afterNavigate(({ from }) => {
		cameFromApp = Boolean(from?.url);
	});

	$effect(() => {
		const current = id;
		const session = getSessionCookieClientSide() ?? '';
		token = session;
		item = null;
		if (!session) return;
		let alive = true;
		fetchMediaDetail(session, current)
			.then((detail) => {
				if (alive) item = detail;
			})
			.catch(() => {
				// The player still works with the session's title.
			});
		return () => {
			alive = false;
		};
	});

	const subheading = $derived(
		item && item.season !== null && item.episode !== null
			? t('player.episode_label', { season: item.season, episode: item.episode })
			: item?.year
				? String(item.year)
				: null
	);

	/** The next episode replaces this page in history: Back leaves the player.
	 * It starts with its own defaults: no query is carried over. */
	function playNext(nextId: string) {
		void goto(resolve('/watch/[id]', { id: nextId }), { replaceState: true });
	}

	function exit() {
		if (cameFromApp && history.length > 1) history.back();
		else void goto(resolve('/media/[id]', { id }));
	}
</script>

<svelte:head><title>{item?.title ?? t('player.title')} · LisekMovie</title></svelte:head>

{#if token !== null}
	{#key id}
		<Player
			{id}
			{token}
			initialAudio={picked.audio}
			initialSubtitle={picked.subtitle}
			startAt={picked.start}
			heading={item?.title ?? null}
			{subheading}
			backdrop={item && token ? imageUrl(item.backdrop_url, token) : null}
			onexit={exit}
			onnext={playNext}
		/>
	{/key}
{:else}
	<div class="fixed inset-0 bg-black"></div>
{/if}
