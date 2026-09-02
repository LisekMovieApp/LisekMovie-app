<!--
  Runda 9: `/library/[id]` — the Navbar's new per-library category links
  (e.g. "Filmy", "Seriale", however the admin named them) need somewhere to
  go. This is the "basic grid" skeleton the request asked for: one
  library's whole browsable contents (movies and series, never bare
  episodes — same grouping the home feed already uses), fetched from
  `GET /api/libraries/{id}/items` in one round trip (see
  `lisek-server::media::library_items`'s doc comment for the exact response
  shape and why the library's own name rides along instead of a second
  fetch).

  Same client-side `onMount` fetch shape as every other data page in this
  app (Home, the Detail View, the Admin Panel's tabs) — see Home's own top
  comment for why this codebase doesn't thread the session token through a
  `+page.ts` load function. Error/not-found handling mirrors the Detail
  View's `fetchMediaDetail`/`MediaNotFoundError` split exactly:
  `fetchLibraryItems` throws `LibraryNotFoundError` specifically for a 404
  (a deleted or mistyped library id) so that case gets its own message and
  a way back, distinct from a generic "couldn't load, try again".

  Reuses `MediaCard` as-is (backdrop art, title overlay, badge pill —
  including a movie's new Runda 9 "Nowość" badge and a series' existing
  ones, both already wired through the shared `to_response` helper on the
  backend) inside a plain wrapping flex row instead of `MediaRow`'s
  `Carousel`: a whole-library browse view is meant to show everything at
  once, not slide through it a row at a time. `MediaCard`'s fixed-width,
  `shrink-0` classes work identically wrapped in a flex row as they do
  inside a carousel strip, so no new card component was needed for this.

  `pt-24` on the wrapper clears the fixed Navbar exactly like
  `/admin/**`'s own layout does (see that layout's comment for the same
  88px-tall-unscrolled math) — this route has no shared layout of its own
  to put that in, so it's applied directly here instead.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchLibraryItems,
		LibraryNotFoundError,
		type LibrarySummary,
		type MediaItem
	} from '$lib/media/api';
	import MediaCard from '$lib/components/MediaCard.svelte';
	import SkeletonCard from '$lib/components/SkeletonCard.svelte';

	const libraryId = $derived(page.params.id ?? '');

	let library = $state<LibrarySummary | null>(null);
	let items: MediaItem[] = $state([]);
	let token = $state('');
	let loading = $state(true);
	let notFound = $state(false);
	let loadFailed = $state(false);

	async function load() {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;

		loading = true;
		notFound = false;
		loadFailed = false;

		try {
			const result = await fetchLibraryItems(sessionToken, libraryId);
			library = result.library;
			items = result.items;
		} catch (err) {
			if (err instanceof LibraryNotFoundError) {
				notFound = true;
			} else {
				loadFailed = true;
			}
		} finally {
			loading = false;
		}
	}

	onMount(load);
</script>

<main class="min-h-screen bg-base px-4 pt-24 pb-16 md:px-8">
	{#if loading}
		<div class="mx-auto flex max-w-6xl flex-wrap gap-3">
			{#each { length: 12 } as _, i (i)}
				<SkeletonCard />
			{/each}
		</div>
	{:else if notFound}
		<div class="flex flex-col items-center gap-4 pt-16 text-center">
			<p class="text-sm text-neutral-400">{t('library.not_found')}</p>
			<a
				href={resolve('/')}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('detail.back_to_home')}
			</a>
		</div>
	{:else if loadFailed}
		<div class="flex flex-col items-center gap-4 pt-16 text-center">
			<p class="text-sm text-red-400">{t('library.load_failed')}</p>
			<button
				type="button"
				onclick={load}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('home.retry')}
			</button>
		</div>
	{:else}
		<div class="mx-auto max-w-6xl">
			<h1 class="text-2xl font-semibold text-neutral-100">{library?.name ?? ''}</h1>

			{#if items.length === 0}
				<p class="mt-8 text-sm text-neutral-400">{t('library.empty')}</p>
			{:else}
				<div class="mt-6 flex flex-wrap gap-3">
					{#each items as item (item.id)}
						<MediaCard {item} {token} />
					{/each}
				</div>
			{/if}
		</div>
	{/if}
</main>
