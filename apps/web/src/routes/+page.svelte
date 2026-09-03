<!--
  Home. Faza 4 shipped the layout with skeletons; Faza 5 wires it to
  `GET /api/home`. Loads client-side via `onMount`, same pattern as the
  Admin Panel's tabs (`getSessionCookieClientSide` + a bare fetch) rather
  than a `+page.ts` load function — see `admin/plugins/+page.svelte`'s own
  comment for why this codebase prefers that over threading the session
  token through another layer of load functions for one page.

  `feed === null` (still loading, or the fetch failed) keeps every child
  component in its existing skeleton state; a successful-but-empty feed
  (nothing scanned into any library yet) replaces the whole page body with
  a friendly "add a library" message instead of an indefinite skeleton —
  the skeleton means "loading", not "empty forever".
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchHomeFeed, type HomeFeed } from '$lib/media/api';
	import HeroBanner from '$lib/components/HeroBanner.svelte';
	import MediaRow from '$lib/components/MediaRow.svelte';
	import Top10Row from '$lib/components/Top10Row.svelte';

	let feed: HomeFeed | null = $state(null);
	let loadFailed = $state(false);
	let token = $state('');

	async function load() {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;

		loadFailed = false;
		try {
			feed = await fetchHomeFeed(sessionToken);
		} catch {
			loadFailed = true;
		}
	}

	onMount(load);

	let isEmpty = $derived.by(() => {
		const loaded = feed;
		if (loaded === null) return false;
		return (
			loaded.hero === null &&
			loaded.recently_added.length === 0 &&
			loaded.top10.length === 0 &&
			loaded.recommended.length === 0 &&
			loaded.popular.length === 0
		);
	});
</script>

{#if loadFailed}
	<main class="flex min-h-screen flex-col items-center justify-center gap-4 bg-base px-4">
		<p class="text-sm text-red-400">{t('home.load_failed')}</p>
		<button
			type="button"
			onclick={load}
			class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
		>
			{t('home.retry')}
		</button>
	</main>
{:else if isEmpty}
	<main
		class="flex min-h-screen flex-col items-center justify-center gap-3 bg-base px-4 text-center"
	>
		<h1 class="text-xl font-semibold text-neutral-100">{t('home.empty_title')}</h1>
		<p class="max-w-sm text-sm text-neutral-400">{t('home.empty_message')}</p>
		{#if page.data.user?.is_admin}
			<a
				href={resolve('/admin/libraries')}
				class="mt-2 rounded-md bg-gold px-4 py-2 text-sm font-medium text-black transition-colors hover:bg-gold-dim"
			>
				{t('home.empty_go_to_libraries')}
			</a>
		{/if}
	</main>
{:else}
	<main class="min-h-screen bg-base pb-16">
		<HeroBanner item={feed?.hero ?? null} {token} />

		<div class="mt-8 flex flex-col gap-8">
			<MediaRow titleKey="home.row_recently_added" items={feed?.recently_added ?? null} {token} />
			<Top10Row items={feed?.top10 ?? null} {token} />
			<MediaRow titleKey="home.row_recommended" items={feed?.recommended ?? null} {token} />
			<MediaRow titleKey="home.row_popular" items={feed?.popular ?? null} {token} />
		</div>
	</main>
{/if}
