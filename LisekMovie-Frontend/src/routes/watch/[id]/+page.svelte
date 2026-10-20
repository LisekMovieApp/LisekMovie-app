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

	const id = $derived(page.params.id ?? '');
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
			heading={item?.title ?? null}
			{subheading}
			backdrop={item && token ? imageUrl(item.backdrop_url, token) : null}
			onexit={exit}
		/>
	{/key}
{:else}
	<div class="fixed inset-0 bg-black"></div>
{/if}
