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

  Faza 6: "Grupuj kolekcje" folds the films of each collection (two or
  more in this library) into one card leading to `/collection/[id]`. On by
  default; the choice is remembered per browser (localStorage). A letter
  filter always lists single films.

  Faza 12: the letter *filter* row is gone. An alpha picker (`AlphaPicker`)
  jumps instead: the server's jump table (`fetchLibraryLetters`, same sort
  and grouping) says where a letter's first title sits in the whole list,
  so a jump loads that page when needed, scrolls the card to the middle of
  the screen and focuses it (the TV remote continues from there). Picking
  a letter while sorted by "recently added" switches to A–Z first. The
  picker highlights the letter of the focused card, or of the first row
  on screen after the viewer scrolls. Shapes: a sticky column on the right
  (desktop, TV), a sticky strip under the top bar (tablet), a "Jump to…"
  pill at the bottom (phone). The grid itself is a CSS grid of posters
  whose column count follows the screen width.
-->
<script lang="ts">
	import { tick } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchLibraryItems,
		fetchLibraryLetters,
		LibraryNotFoundError,
		type LibraryLetter,
		type LibrarySort,
		type LibrarySummary,
		type MediaItem
	} from '$lib/media/api';
	import { jumpTarget, letterAt } from '$lib/media/alpha';
	import PosterCard from './PosterCard.svelte';
	import AlphaPicker from './AlphaPicker.svelte';
	import Icon from './Icon.svelte';

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

	let token = $state('');
	let pageNum = $state(1);
	let sort = $state<LibrarySort>('title_asc');
	let viewDensity = $state<'comfortable' | 'compact'>('comfortable');
	const GROUP_KEY = 'lisek.library.groupCollections';
	function storedGrouping(): boolean {
		try {
			return localStorage.getItem(GROUP_KEY) !== '0';
		} catch {
			return true;
		}
	}
	let groupCollections = $state(storedGrouping());
	let items: MediaItem[] | null = $state(null);
	/** `null` until the first page of this library has loaded. */
	let total = $state<number | null>(null);
	let pageSize = $state(100);
	let error = $state(false);

	/** The alpha picker's jump table and the letter it highlights. */
	let letters = $state<LibraryLetter[]>([]);
	let activeLetter = $state<string | null>(null);
	const available = $derived(new Set(letters.map((l) => l.letter)));
	let grid = $state<HTMLElement>();

	/** Same role as `/library/[id]/+page.svelte`'s own `loadSeq` — see that
	 * file's top comment. */
	let loadSeq = 0;
	let lettersSeq = 0;

	async function loadPage(id: string, p: number, s: LibrarySort, mySeq: number) {
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		token = sessionToken;

		try {
			const result = await fetchLibraryItems(sessionToken, id, {
				page: p,
				sort: s,
				groupCollections
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

	/** The jump table for the order on screen ("recently added" has no
	 * letters: a pick switches to A–Z, so that table is loaded). */
	async function loadLetters(id: string, s: LibrarySort, grouped: boolean) {
		const mySeq = ++lettersSeq;
		const sessionToken = getSessionCookieClientSide();
		if (!sessionToken) return;
		try {
			const result = await fetchLibraryLetters(sessionToken, id, {
				sort: s === 'recent' ? 'title_asc' : s,
				groupCollections: grouped
			});
			if (mySeq === lettersSeq) letters = result;
		} catch {
			// No picker: the grid still pages.
			if (mySeq === lettersSeq) letters = [];
		}
	}

	$effect(() => {
		const id = libraryId; // read synchronously — this is what Svelte tracks
		pageNum = 1;
		sort = 'title_asc';
		items = null;
		total = null;
		error = false;
		letters = [];
		activeLetter = null;
		const mySeq = ++loadSeq;
		loadPage(id, 1, 'title_asc', mySeq);
		loadLetters(id, 'title_asc', groupCollections);
	});

	function goToPage(next: number) {
		if (next < 1) return;
		pageNum = next;
		items = null;
		const mySeq = ++loadSeq;
		loadPage(libraryId, next, sort, mySeq);
	}

	function changeSort(next: LibrarySort) {
		const lettersChange = (next === 'title_desc') !== (sort === 'title_desc');
		sort = next;
		pageNum = 1;
		items = null;
		activeLetter = null;
		const mySeq = ++loadSeq;
		loadPage(libraryId, 1, next, mySeq);
		if (lettersChange) loadLetters(libraryId, next, groupCollections);
	}

	function toggleGrouping() {
		groupCollections = !groupCollections;
		try {
			localStorage.setItem(GROUP_KEY, groupCollections ? '1' : '0');
		} catch {
			// Private mode: the choice lasts for this page only.
		}
		pageNum = 1;
		items = null;
		activeLetter = null;
		const mySeq = ++loadSeq;
		loadPage(libraryId, 1, sort, mySeq);
		loadLetters(libraryId, sort, groupCollections);
	}

	function cards(): HTMLElement[] {
		return grid ? Array.from(grid.querySelectorAll<HTMLElement>(':scope > a')) : [];
	}

	/** Jumps to `letter`'s first title: loads its page when it is not the
	 * one on screen, then scrolls the card to the middle and focuses it. */
	async function jump(letter: string) {
		const switchToAz = sort === 'recent';
		const target = jumpTarget(letters, letter, pageSize);
		if (!target) return;
		activeLetter = letter;
		manualScroll = false;
		if (switchToAz || target.page !== pageNum || items === null) {
			if (switchToAz) sort = 'title_asc';
			pageNum = target.page;
			items = null;
			const mySeq = ++loadSeq;
			await loadPage(libraryId, target.page, sort, mySeq);
			if (mySeq !== loadSeq) return;
		}
		await tick();
		const card = cards()[target.offset];
		if (!card) return;
		card.focus({ preventScroll: true });
		const reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
		card.scrollIntoView?.({ block: 'center', behavior: reduced ? 'auto' : 'smooth' });
	}

	/** Highlight follows the focused card (remote, keyboard). */
	function onGridFocus(event: FocusEvent) {
		if (sort === 'recent' || !(event.target instanceof HTMLElement)) return;
		const index = cards().indexOf(event.target);
		if (index >= 0) activeLetter = letterAt(letters, (pageNum - 1) * pageSize + index);
	}

	/** …and the first row on screen once the viewer scrolls by hand. A
	 * jump's own smooth scroll is not "by hand": the row above the target
	 * may still belong to the previous letter. */
	let manualScroll = false;
	let scrollFrame = 0;
	function onManualScroll() {
		manualScroll = true;
	}
	function onScroll() {
		if (!manualScroll || scrollFrame || sort === 'recent' || !letters.length) return;
		scrollFrame = requestAnimationFrame(() => {
			scrollFrame = 0;
			const top = 112; // under the top bar (and the tablet strip)
			const index = cards().findIndex((card) => card.getBoundingClientRect().top >= top);
			if (index >= 0) activeLetter = letterAt(letters, (pageNum - 1) * pageSize + index);
		});
	}

	const showPicker = $derived(letters.length > 0 && (total ?? 0) > 0);
	let totalPages = $derived(total ? Math.ceil(total / pageSize) : 0);
	let rangeFrom = $derived(!total ? 0 : (pageNum - 1) * pageSize + 1);
	let rangeTo = $derived(Math.min(pageNum * pageSize, total ?? 0));

	const columns = $derived(
		viewDensity === 'compact'
			? 'grid-cols-[repeat(auto-fill,minmax(5.25rem,1fr))] sm:grid-cols-[repeat(auto-fill,minmax(6.5rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(7.5rem,1fr))]'
			: 'grid-cols-[repeat(auto-fill,minmax(6.75rem,1fr))] sm:grid-cols-[repeat(auto-fill,minmax(8.5rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] 2xl:grid-cols-[repeat(auto-fill,minmax(10.5rem,1fr))]'
	);
	const pagerButton =
		'rounded-md border border-neutral-700 px-2.5 py-1 text-sm text-neutral-200 transition-colors hover:border-gold-dim focus-visible:border-gold focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-30';
	const chip =
		'rounded-md border px-2 py-1.5 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-gold';
</script>

<svelte:window
	onscroll={onScroll}
	onwheel={onManualScroll}
	ontouchmove={onManualScroll}
	onkeydown={(event) => {
		if (['PageDown', 'PageUp', 'Home', 'End', ' '].includes(event.key)) onManualScroll();
	}}
/>

<section class="px-4 pt-4 pb-4 md:px-8">
	<h2 class="mb-4 text-lg font-semibold text-neutral-100">
		{t('library.all_items_title', { name: libraryName })}
	</h2>

	{#if error}
		<p class="text-sm text-red-400">{t('library.items_failed')}</p>
	{:else if total === null}
		<div class="grid gap-x-3 gap-y-5 {columns}">
			{#each { length: 12 } as _, i (i)}
				<div class="aspect-[2/3] animate-pulse rounded-md bg-surface"></div>
			{/each}
		</div>
	{:else if total === 0}
		<p class="text-sm text-neutral-400">{t('library.empty')}</p>
	{:else}
		<div class="mb-4 flex flex-wrap items-center gap-x-3 gap-y-2">
			<p class="text-sm text-neutral-400 tabular-nums">
				{t('library.showing_range', { from: rangeFrom, to: rangeTo, total })}
			</p>

			<div class="flex items-center gap-1">
				<button
					type="button"
					disabled={pageNum <= 1}
					onclick={() => goToPage(pageNum - 1)}
					aria-label={t('library.prev')}
					class={pagerButton}
				>
					←
				</button>
				<button
					type="button"
					disabled={pageNum >= totalPages}
					onclick={() => goToPage(pageNum + 1)}
					aria-label={t('library.next')}
					class={pagerButton}
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
					class="{chip} {viewDensity === 'comfortable'
						? 'border-gold text-gold'
						: 'border-neutral-700 text-neutral-200 hover:border-gold-dim'}"
				>
					{t('library.view_comfortable')}
				</button>
				<button
					type="button"
					onclick={() => (viewDensity = 'compact')}
					aria-pressed={viewDensity === 'compact'}
					class="{chip} {viewDensity === 'compact'
						? 'border-gold text-gold'
						: 'border-neutral-700 text-neutral-200 hover:border-gold-dim'}"
				>
					{t('library.view_compact')}
				</button>
			</div>

			<button
				type="button"
				onclick={toggleGrouping}
				aria-pressed={groupCollections}
				class="{chip} flex items-center gap-1.5 {groupCollections
					? 'border-gold text-gold'
					: 'border-neutral-700 text-neutral-200 hover:border-gold-dim'}"
			>
				<Icon name="stack" class="size-4" />
				{t('collections.group_toggle')}
			</button>
		</div>

		{#if showPicker}
			<!-- Tablet: a strip pinned under the top bar. -->
			<div
				class="sticky top-16 z-30 -mx-4 mb-4 hidden border-b border-white/5 bg-base/85 px-4 py-2 backdrop-blur-md sm:block md:top-[4.5rem] md:-mx-8 md:px-8 lg:hidden"
			>
				<AlphaPicker {available} active={activeLetter} orientation="horizontal" onpick={jump} />
			</div>
		{/if}

		<div class="lg:flex lg:items-start lg:gap-5">
			<div class="min-w-0 flex-1">
				{#if items === null}
					<div class="grid gap-x-3 gap-y-5 {columns}">
						{#each { length: 12 } as _, i (i)}
							<div class="aspect-[2/3] animate-pulse rounded-md bg-surface"></div>
						{/each}
					</div>
				{:else}
					<div bind:this={grid} onfocusin={onGridFocus} class="grid gap-x-3 gap-y-5 {columns}">
						{#each items as item (item.id)}
							<PosterCard {item} {token} fluid />
						{/each}
					</div>
				{/if}

				{#if totalPages > 1}
					<div class="mt-6 flex items-center justify-center gap-3">
						<button
							type="button"
							disabled={pageNum <= 1}
							onclick={() => goToPage(pageNum - 1)}
							aria-label={t('library.prev')}
							class={pagerButton}
						>
							←
						</button>
						<p class="text-sm text-neutral-400 tabular-nums">
							{t('library.showing_range', { from: rangeFrom, to: rangeTo, total })}
						</p>
						<button
							type="button"
							disabled={pageNum >= totalPages}
							onclick={() => goToPage(pageNum + 1)}
							aria-label={t('library.next')}
							class={pagerButton}
						>
							→
						</button>
					</div>
				{/if}
			</div>

			{#if showPicker}
				<!-- Desktop and TV: a column at the right edge, Jellyfin-style. -->
				<aside
					class="sticky top-[5.5rem] hidden h-[calc(100dvh-7rem)] max-h-[46rem] shrink-0 self-start lg:block"
				>
					<AlphaPicker {available} active={activeLetter} orientation="vertical" onpick={jump} />
				</aside>
			{/if}
		</div>

		{#if showPicker}
			<!-- Phone: a "Jump to…" pill that stays at the bottom of the screen. -->
			<div class="pointer-events-none sticky bottom-4 z-30 mt-4 flex justify-end sm:hidden">
				<div class="pointer-events-auto">
					<AlphaPicker {available} active={activeLetter} orientation="menu" onpick={jump} />
				</div>
			</div>
		{/if}
	{/if}
</section>
