<!--
  Runda 10 — `/library/[id]`'s "Wszystkie" (all items) section: a real,
  paginated, sortable, letter-filterable grid, functionally inspired by
  Jellyfin's own library browse view but NOT copying its markup — a
  scaled-down, honestly-scoped version of it (real pagination/counter/
  prev-next/sort/letter-filter/view-density toggle; Shuffle is present but
  honestly disabled, same convention as every other Play button in this
  app, since no playback engine exists yet). See the Runda 10 plan for the
  deliberate, documented scope limits: only 3 sort options, plain ASCII A-Z
  letter buttons (a title starting with a non-Latin character isn't
  reachable by any single letter button — only "Wszystkie" reaches it — an
  honest limitation, not a bug), and a 2-state density toggle rather than a
  full view-mode picker.

  Runda 11: "Odtwórz wszystko" (Play All) was removed entirely rather than
  kept as a second honestly-disabled button — the user's call, not a
  playback-engine change; Shuffle stays exactly as it was. The outer
  section also dropped `mx-auto max-w-6xl`, which centered this whole
  section (heading, toolbar, letter row) in a narrower column than the
  `MediaRow` sections above it on wide screens — now it's flush with the
  same `px-4 md:px-8` left edge the rest of the page uses.

  Owns its own independent data-loading: an `$effect` keyed ONLY on
  `libraryId` resets page/sort/letter to their defaults and loads page 1
  whenever the library itself changes; every other change (prev/next, sort,
  a letter button, the density toggle) is a plain imperative handler, never
  a second `$effect` that reads the same state it writes — that shape would
  be a second copy of the exact reactivity bug this whole round fixes (see
  the plan's own callout). `loadSeq` guards every fetch the same way
  `/library/[id]/+page.svelte`'s own `triggerLoad`/`loadForId` do, so a
  slow, superseded request can never clobber a newer one's results.

  Resolves its own session token from the cookie (same self-contained
  pattern every page in this app already uses — see `+page.svelte` (home)'s
  own comment) rather than taking it as a prop from the parent, specifically
  to avoid any prop-timing race with the parent's own token resolution.

  `onLibrary`/`onNotFound` report this section's own successful/`404`
  outcome back up to `/library/[id]/+page.svelte` — since Recent/
  Recommended/Popular/Wszystkie are four fully independent requests (see the
  Runda 10 plan's reasoning for why they're separate endpoints rather than
  one bundled response), whichever of the four resolves first can populate
  the shared header, and any one of them hitting a real 404 is enough to
  know the library itself doesn't exist.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchLibraryItems,
		LibraryNotFoundError,
		type LibrarySort,
		type LibrarySummary,
		type MediaItem
	} from '$lib/media/api';
	import MediaCard from './MediaCard.svelte';
	import SkeletonCard from './SkeletonCard.svelte';

	interface Props {
		libraryId: string;
		/** Already-resolved library name for this section's own "Wszystkie
		 * — {name}" heading — the parent only renders this component once
		 * its shared header is ready, so this is never blank/stale here. */
		libraryName: string;
		onLibrary?: (library: LibrarySummary) => void;
		onNotFound?: () => void;
	}

	let { libraryId, libraryName, onLibrary, onNotFound }: Props = $props();

	const LETTERS = Array.from({ length: 26 }, (_, i) => String.fromCharCode(65 + i));

	let token = $state('');
	let pageNum = $state(1);
	let sort = $state<LibrarySort>('title_asc');
	let letter = $state<string | null>(null);
	let viewDensity = $state<'comfortable' | 'compact'>('comfortable');
	let items: MediaItem[] | null = $state(null);
	let total = $state(0);
	let pageSize = $state(100);
	let error = $state(false);

	/** Same role as `/library/[id]/+page.svelte`'s own `loadSeq` — see that
	 * file's top comment. */
	let loadSeq = 0;

	async function loadPage(id: string, p: number, s: LibrarySort, l: string | null, mySeq: number) {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;

		try {
			const result = await fetchLibraryItems(sessionToken, id, {
				page: p,
				sort: s,
				letter: l ?? undefined
			});
			if (mySeq !== loadSeq) return; // superseded — discard
			items = result.items;
			total = result.total;
			pageSize = result.page_size;
			error = false;
			onLibrary?.(result.library);
		} catch (err) {
			if (mySeq !== loadSeq) return; // superseded — discard
			if (err instanceof LibraryNotFoundError) {
				onNotFound?.();
			}
			error = true;
			items = [];
		}
	}

	$effect(() => {
		const id = libraryId; // read synchronously — this is what Svelte tracks
		pageNum = 1;
		sort = 'title_asc';
		letter = null;
		items = null;
		error = false;
		const mySeq = ++loadSeq;
		loadPage(id, 1, 'title_asc', null, mySeq);
	});

	function goToPage(next: number) {
		if (next < 1) return;
		pageNum = next;
		items = null;
		const mySeq = ++loadSeq;
		loadPage(libraryId, next, sort, letter, mySeq);
	}

	function changeSort(next: LibrarySort) {
		sort = next;
		pageNum = 1;
		items = null;
		const mySeq = ++loadSeq;
		loadPage(libraryId, 1, next, letter, mySeq);
	}

	function changeLetter(next: string | null) {
		letter = next;
		pageNum = 1;
		items = null;
		const mySeq = ++loadSeq;
		loadPage(libraryId, 1, sort, next, mySeq);
	}

	let totalPages = $derived(total > 0 ? Math.ceil(total / pageSize) : 0);
	let rangeFrom = $derived(total === 0 ? 0 : (pageNum - 1) * pageSize + 1);
	let rangeTo = $derived(Math.min(pageNum * pageSize, total));
</script>

<section class="px-4 pt-4 pb-4 md:px-8">
	<h2 class="mb-4 text-lg font-semibold text-neutral-100">
		{t('library.all_items_title', { name: libraryName })}
	</h2>

	{#if error}
		<p class="text-sm text-red-400">{t('library.items_failed')}</p>
	{:else if items === null}
		<div class="flex flex-wrap gap-3">
			{#each { length: 12 } as _, i (i)}
				<SkeletonCard />
			{/each}
		</div>
	{:else if total === 0}
		<p class="text-sm text-neutral-400">
			{letter ? t('library.no_letter_matches') : t('library.empty')}
		</p>
	{:else}
		<div class="mb-4 flex flex-wrap items-center gap-3">
			<p class="text-sm text-neutral-400">
				{t('library.showing_range', { from: rangeFrom, to: rangeTo, total })}
			</p>

			<div class="flex items-center gap-1">
				<button
					type="button"
					disabled={pageNum <= 1}
					onclick={() => goToPage(pageNum - 1)}
					aria-label={t('library.prev')}
					class="rounded-md border border-neutral-700 px-2 py-1 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-30"
				>
					←
				</button>
				<button
					type="button"
					disabled={pageNum >= totalPages}
					onclick={() => goToPage(pageNum + 1)}
					aria-label={t('library.next')}
					class="rounded-md border border-neutral-700 px-2 py-1 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-30"
				>
					→
				</button>
			</div>

			<button
				type="button"
				disabled
				title={t('library.playback_unavailable')}
				class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 disabled:cursor-not-allowed disabled:opacity-50"
			>
				🔀 {t('library.shuffle')}
			</button>

			<select
				value={sort}
				onchange={(e) => changeSort((e.currentTarget as HTMLSelectElement).value as LibrarySort)}
				aria-label={t('library.sort_label')}
				class="rounded-md border border-neutral-700 bg-surface px-2 py-1.5 text-sm text-neutral-200"
			>
				<option value="title_asc">{t('library.sort_title_asc')}</option>
				<option value="title_desc">{t('library.sort_title_desc')}</option>
				<option value="recent">{t('library.sort_recent')}</option>
			</select>

			<div class="flex items-center gap-1">
				<button
					type="button"
					onclick={() => (viewDensity = 'comfortable')}
					aria-pressed={viewDensity === 'comfortable'}
					class="rounded-md border px-2 py-1.5 text-sm transition-colors {viewDensity ===
					'comfortable'
						? 'border-gold text-gold'
						: 'border-neutral-700 text-neutral-200 hover:border-gold-dim'}"
				>
					{t('library.view_comfortable')}
				</button>
				<button
					type="button"
					onclick={() => (viewDensity = 'compact')}
					aria-pressed={viewDensity === 'compact'}
					class="rounded-md border px-2 py-1.5 text-sm transition-colors {viewDensity === 'compact'
						? 'border-gold text-gold'
						: 'border-neutral-700 text-neutral-200 hover:border-gold-dim'}"
				>
					{t('library.view_compact')}
				</button>
			</div>
		</div>

		<div class="mb-4 flex flex-wrap gap-1.5">
			<button
				type="button"
				onclick={() => changeLetter(null)}
				class="rounded px-2 py-1 text-xs font-medium transition-colors {letter === null
					? 'bg-gold text-black'
					: 'bg-surface text-neutral-300 hover:bg-white/10'}"
			>
				{t('library.letter_all')}
			</button>
			{#each LETTERS as l (l)}
				<button
					type="button"
					onclick={() => changeLetter(l)}
					class="rounded px-2 py-1 text-xs font-medium transition-colors {letter === l
						? 'bg-gold text-black'
						: 'bg-surface text-neutral-300 hover:bg-white/10'}"
				>
					{l}
				</button>
			{/each}
		</div>

		<div class="flex flex-wrap gap-3">
			{#each items as item (item.id)}
				<MediaCard {item} {token} size={viewDensity} />
			{/each}
		</div>
	{/if}
</section>
