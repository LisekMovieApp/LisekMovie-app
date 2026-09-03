<!--
  Runda 9 introduced `/library/[id]` as one simple grid of a library's whole
  contents. Runda 10 rebuilds it into a proper per-category mini-homepage —
  no large hero (unlike `/+page.svelte`, this is meant to be utilitarian:
  just a header, then straight into content), reusing exactly the home
  feed's own mechanisms wherever one already exists:

    {library.name}
    Ostatnio dodane — {library.name}      ← MediaRow, same `list_recently_added`
    Polecane dla Ciebie — {library.name}  ← MediaRow, same `list_recommended`
    Popularne teraz — {library.name}      ← MediaRow, same `list_popular`
    Wszystkie — {library.name}            ← LibraryAllSection (paginated grid)

  Every section is strictly scoped to this one library — each of the four
  backing endpoints (`GET /api/libraries/{id}/recent|recommended|popular|
  items`) filters server-side (see `lisek_db`'s nullable `library_id: Option
  <Uuid>` bind on `list_recently_added`/`list_recommended`/`list_popular`,
  and `list_items_by_library_page`'s hard `library_id` filter) — nothing
  here ever fetches the whole catalog and slices it client-side. Series
  always show as grouped shows (never raw episodes) in every section,
  because every one of those four queries is built on the same
  `home_candidates` CTE the home feed itself uses.

  ROUTING BUG FIX (Runda 10): this file used to do
  `const libraryId = $derived(page.params.id ?? ''); onMount(load);` —
  `onMount` fires exactly once per component instance, but SvelteKit
  reuses this exact instance across a client-side navigation between two
  `/library/[id]` URLs that only differ in `id` (e.g. clicking "Filmy" then
  "Seriale" in the Navbar), so `load()` never re-ran and the page kept
  showing the previous library's content under the new URL until a full
  refresh. Fixed with a Svelte 5 `$effect` that reads the reactive
  `libraryId` synchronously — an effect re-runs whenever a value it read
  synchronously on its last run changes, including on the very first mount,
  so it fully replaces `onMount` here. `triggerLoad` resets every bit of
  state SYNCHRONOUSLY (before any fetch starts) so the previous library's
  content is never visible under the new URL, not even for one frame, and
  `loadSeq` discards a slower, superseded request's result if it resolves
  after a newer navigation already landed (rapid Filmy→Seriale→Anime
  clicking). The identical fix was applied to `/media/[id]/+page.svelte`
  (same latent bug, found during this round's audit).

  Recent/Recommended/Popular are three fully independent requests (not one
  bundled response) specifically so a failure in one (e.g. "Popularne")
  can never take another down with it — this codebase has no existing
  "partial success" JSON convention, and three real, independent HTTP
  requests get that isolation for free. Each carries its own `library` in
  its response, so whichever of the three resolves first can fill in this
  page's shared header — that's why the header + all three rows stay
  behind one shared skeleton (`library === null`) until any of them lands,
  rather than each row appearing with a blank "— " title before the name is
  known. If all three fail outright (e.g. the whole server is unreachable),
  `loadFailed` falls back to a page-level retry rather than leaving the
  skeleton up forever.

  `LibraryAllSection` (the "Wszystkie" grid) is a separate component
  precisely because it owns real pagination/sort/letter-filter state that
  would otherwise bloat this file — see its own top comment for its data
  flow and its own `$effect`/`loadSeq` pair, which mirrors this file's
  exactly.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchLibraryRecent,
		fetchLibraryRecommended,
		fetchLibraryPopular,
		LibraryNotFoundError,
		type LibraryRowItems,
		type LibrarySummary,
		type MediaItem
	} from '$lib/media/api';
	import MediaRow from '$lib/components/MediaRow.svelte';
	import LibraryAllSection from '$lib/components/LibraryAllSection.svelte';
	import SkeletonCard from '$lib/components/SkeletonCard.svelte';

	const libraryId = $derived(page.params.id ?? '');

	let token = $state('');
	let library = $state<LibrarySummary | null>(null);
	let notFound = $state(false);
	/** Only ever true once every row fetch for the current `libraryId` has
	 * settled with nothing usable — see `maybeGiveUp` below. */
	let loadFailed = $state(false);

	let recentItems: MediaItem[] | null = $state(null);
	let recentError = $state(false);
	let recommendedItems: MediaItem[] | null = $state(null);
	let recommendedError = $state(false);
	let popularItems: MediaItem[] | null = $state(null);
	let popularError = $state(false);

	/** How many of the three row fetches have settled (success or failure)
	 * for the current `libraryId` — reset to 0 on every navigation, used
	 * only by `maybeGiveUp` to detect "every row failed, we never even
	 * learned the library's name" so the page doesn't show a loading
	 * skeleton forever. */
	let rowsSettled = $state(0);

	/** Bumped once per navigation; guards every state write below against
	 * an older, superseded navigation's response landing after a newer one
	 * already has. */
	let loadSeq = 0;

	function handleLibrary(resolved: LibrarySummary) {
		if (!library) library = resolved;
		notFound = false;
		loadFailed = false;
	}

	function handleNotFound() {
		notFound = true;
	}

	function maybeGiveUp(mySeq: number) {
		if (mySeq !== loadSeq) return;
		if (library !== null || notFound) return;
		if (rowsSettled >= 3) loadFailed = true;
	}

	async function loadRow(
		id: string,
		mySeq: number,
		fetcher: (sessionToken: string, libraryId: string) => Promise<LibraryRowItems>,
		setItems: (items: MediaItem[]) => void,
		setError: (value: boolean) => void
	) {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;

		try {
			const result = await fetcher(sessionToken, id);
			if (mySeq !== loadSeq) return; // superseded — discard
			setItems(result.items);
			setError(false);
			handleLibrary(result.library);
		} catch (err) {
			if (mySeq !== loadSeq) return; // superseded — discard
			if (err instanceof LibraryNotFoundError) {
				handleNotFound();
			} else {
				setError(true);
				setItems([]);
			}
		} finally {
			if (mySeq === loadSeq) {
				rowsSettled += 1;
				maybeGiveUp(mySeq);
			}
		}
	}

	function triggerLoad(id: string) {
		const mySeq = ++loadSeq;
		// Reset SYNCHRONOUSLY, before any fetch starts, so the previous
		// library's content is never visible under the new URL, even for
		// one frame.
		library = null;
		notFound = false;
		loadFailed = false;
		recentItems = null;
		recentError = false;
		recommendedItems = null;
		recommendedError = false;
		popularItems = null;
		popularError = false;
		rowsSettled = 0;

		loadRow(
			id,
			mySeq,
			fetchLibraryRecent,
			(v) => (recentItems = v),
			(v) => (recentError = v)
		);
		loadRow(
			id,
			mySeq,
			fetchLibraryRecommended,
			(v) => (recommendedItems = v),
			(v) => (recommendedError = v)
		);
		loadRow(
			id,
			mySeq,
			fetchLibraryPopular,
			(v) => (popularItems = v),
			(v) => (popularError = v)
		);
	}

	$effect(() => {
		const id = libraryId; // read synchronously — this is what Svelte tracks
		triggerLoad(id);
	});
</script>

<main class="min-h-screen bg-base pt-24 pb-16">
	{#if notFound}
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
				onclick={() => triggerLoad(libraryId)}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
			>
				{t('home.retry')}
			</button>
		</div>
	{:else if library === null}
		<div class="mx-auto max-w-6xl px-4 md:px-8">
			<div class="mb-6 h-8 w-48 animate-pulse rounded bg-surface"></div>
			<div class="flex flex-wrap gap-3">
				{#each { length: 12 } as _, i (i)}
					<SkeletonCard />
				{/each}
			</div>
		</div>
	{:else}
		<div class="mx-auto max-w-6xl px-4 md:px-8">
			<h1 class="text-2xl font-semibold text-neutral-100">{library.name}</h1>
		</div>

		<div class="mt-6 flex flex-col gap-8">
			<div>
				<MediaRow
					titleKey="library.row_recently_added"
					titleParams={{ name: library.name }}
					items={recentItems}
					{token}
				/>
				{#if recentError}
					<p class="px-4 text-sm text-red-400 md:px-8">{t('library.recent_failed')}</p>
				{/if}
			</div>

			<div>
				<MediaRow
					titleKey="library.row_recommended"
					titleParams={{ name: library.name }}
					items={recommendedItems}
					{token}
				/>
				{#if recommendedError}
					<p class="px-4 text-sm text-red-400 md:px-8">{t('library.recommended_failed')}</p>
				{/if}
			</div>

			<div>
				<MediaRow
					titleKey="library.row_popular"
					titleParams={{ name: library.name }}
					items={popularItems}
					{token}
				/>
				{#if popularError}
					<p class="px-4 text-sm text-red-400 md:px-8">{t('library.popular_failed')}</p>
				{/if}
			</div>

			<LibraryAllSection
				{libraryId}
				libraryName={library.name}
				onLibrary={handleLibrary}
				onNotFound={handleNotFound}
			/>
		</div>
	{/if}
</main>
