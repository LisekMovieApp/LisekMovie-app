// Runda 10 — the regression test the routing-bug report explicitly asked
// for: proof that navigating between two `/library/[id]` URLs (Filmy →
// Seriale, say) without a full page reload actually changes the rendered
// content, not just the URL. This is this project's first test that
// mounts a real `.svelte` component (see `vite.config.ts`'s "client"
// project and `vitest-setup-client.ts`).
//
// `$app/state` is replaced with `createMockPageState` (see
// `$lib/testing/mockPageState.svelte`'s own comment for exactly why
// mutating its `params` object behaves like a real client-side
// navigation). The mock module is built via a dynamic `import()` *inside*
// `vi.mock`'s factory, rather than a same-file top-level `const`, because
// Vitest hoists every `vi.mock` call above the rest of the file — closing
// over a same-file `const` there hits a "cannot access before
// initialization" error; a dynamic import inside the (lazily-evaluated)
// factory sidesteps that entirely. Every `$lib/media/api` fetch this page
// (and the child `LibraryAllSection` it renders once a library resolves)
// calls is replaced with a `vi.fn()` — declared via `vi.hoisted` for the
// same reason — so these tests never touch the network and can
// deterministically control when each request "resolves".
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { flushSync } from 'svelte';
import { render, screen, waitFor } from '@testing-library/svelte';
import LibraryPage from './+page.svelte';
import { page as mockPage } from '$app/state';
import type { LibraryItems, LibraryRowItems, MediaItem } from '$lib/media/api';

vi.mock('$app/state', async () => {
	const { createMockPageState } = await import('$lib/testing/mockPageState.svelte');
	const { page } = createMockPageState({ id: 'lib-a' });
	return { page };
});

vi.mock('$app/paths', () => ({
	resolve: (route: string, params?: Record<string, string>) =>
		route.replace(/\[(\w+)\]/g, (_match, key: string) => params?.[key] ?? ''),
	base: '',
	assets: ''
}));

const { fetchLibraryRecent, fetchLibraryRecommended, fetchLibraryPopular, fetchLibraryItems } =
	vi.hoisted(() => ({
		fetchLibraryRecent: vi.fn(),
		fetchLibraryRecommended: vi.fn(),
		fetchLibraryPopular: vi.fn(),
		fetchLibraryItems: vi.fn()
	}));

vi.mock('$lib/media/api', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/media/api')>();
	return {
		...actual,
		fetchLibraryRecent,
		fetchLibraryRecommended,
		fetchLibraryPopular,
		fetchLibraryItems
	};
});

// `page.params` is the same reactive object every time this getter is
// read (see `mockPageState.svelte`'s own comment) — mutating it through
// this alias is exactly how these tests simulate a client-side navigation.
const mockParams = mockPage.params as { id: string };

function rowFixture(id: string, name: string): LibraryRowItems {
	return { library: { id, name, library_type: 'movies' }, items: [] };
}

function itemsFixture(id: string, name: string): LibraryItems {
	return {
		library: { id, name, library_type: 'movies' },
		items: [],
		total: 0,
		page: 1,
		page_size: 100
	};
}

function mediaItemFixture(id: string, title: string): MediaItem {
	return {
		id,
		item_type: 'movie',
		title,
		year: 2020,
		overview: null,
		season: null,
		episode: null,
		poster_url: `/api/media/${id}/poster`,
		backdrop_url: `/api/media/${id}/backdrop`,
		logo_url: `/api/media/${id}/logo`,
		badge: null
	};
}

/** Runda 11 — a non-empty "Wszystkie" page, so `LibraryAllSection` renders
 * its real toolbar/letter-row/grid branch instead of the empty-library
 * message `itemsFixture` above (`total: 0`) exercises. */
function nonEmptyItemsFixture(id: string, name: string): LibraryItems {
	return {
		library: { id, name, library_type: 'movies' },
		items: [mediaItemFixture(`${id}-item-1`, 'Testowy Film')],
		total: 1,
		page: 1,
		page_size: 100
	};
}

/** Never-resolving promise — stands in for "still in flight" without this
 * test needing to await it. */
function pending<T>(): Promise<T> {
	return new Promise<T>(() => {});
}

describe('/library/[id] page', () => {
	beforeEach(() => {
		fetchLibraryRecent.mockReset();
		fetchLibraryRecommended.mockReset();
		fetchLibraryPopular.mockReset();
		fetchLibraryItems.mockReset();
		document.cookie = 'lisek_session=test-token; Path=/';
		mockParams.id = 'lib-a';
	});

	it('fetches the library present at mount', async () => {
		fetchLibraryRecent.mockResolvedValue(rowFixture('lib-a', 'Filmy'));
		fetchLibraryRecommended.mockResolvedValue(rowFixture('lib-a', 'Filmy'));
		fetchLibraryPopular.mockResolvedValue(rowFixture('lib-a', 'Filmy'));
		fetchLibraryItems.mockResolvedValue(itemsFixture('lib-a', 'Filmy'));

		render(LibraryPage);

		await waitFor(() => expect(fetchLibraryRecent).toHaveBeenCalledWith('test-token', 'lib-a'));
		expect(fetchLibraryRecommended).toHaveBeenCalledWith('test-token', 'lib-a');
		expect(fetchLibraryPopular).toHaveBeenCalledWith('test-token', 'lib-a');
		expect(await screen.findByRole('heading', { level: 1, name: 'Filmy' })).toBeTruthy();
	});

	it('re-fetches and clears stale content when the id changes, without unmounting the page', async () => {
		fetchLibraryRecent.mockResolvedValueOnce(rowFixture('lib-a', 'Filmy'));
		fetchLibraryRecommended.mockResolvedValueOnce(rowFixture('lib-a', 'Filmy'));
		fetchLibraryPopular.mockResolvedValueOnce(rowFixture('lib-a', 'Filmy'));
		fetchLibraryItems.mockResolvedValueOnce(itemsFixture('lib-a', 'Filmy'));

		render(LibraryPage);
		expect(await screen.findByRole('heading', { level: 1, name: 'Filmy' })).toBeTruthy();

		let resolveRecentB: (value: LibraryRowItems) => void = () => {};
		fetchLibraryRecent.mockReturnValueOnce(new Promise((resolve) => (resolveRecentB = resolve)));
		fetchLibraryRecommended.mockReturnValueOnce(pending());
		fetchLibraryPopular.mockReturnValueOnce(pending());
		fetchLibraryItems.mockReturnValueOnce(pending());

		// Simulates clicking "Seriale" in the Navbar after "Filmy" — a
		// client-side navigation to a new `/library/[id]` URL that reuses
		// this exact component instance (never unmounted here).
		flushSync(() => {
			mockParams.id = 'lib-b';
		});

		expect(fetchLibraryRecent).toHaveBeenCalledWith('test-token', 'lib-b');
		// The core assertion: Filmy's title is gone from the DOM the
		// instant the id changed — BEFORE lib-b's request has resolved at
		// all. A page that only changed the URL without re-fetching, or
		// that left the old content on screen while loading, fails here.
		expect(screen.queryByRole('heading', { level: 1, name: 'Filmy' })).toBeNull();

		resolveRecentB(rowFixture('lib-b', 'Seriale'));
		expect(await screen.findByRole('heading', { level: 1, name: 'Seriale' })).toBeTruthy();
	});

	it('discards a stale response from an id that is no longer current', async () => {
		let resolveRecentA: (value: LibraryRowItems) => void = () => {};
		fetchLibraryRecent.mockReturnValueOnce(new Promise((resolve) => (resolveRecentA = resolve)));
		fetchLibraryRecommended.mockReturnValueOnce(pending());
		fetchLibraryPopular.mockReturnValueOnce(pending());
		fetchLibraryItems.mockReturnValueOnce(pending());

		render(LibraryPage);

		fetchLibraryRecent.mockResolvedValueOnce(rowFixture('lib-b', 'Seriale'));
		fetchLibraryRecommended.mockResolvedValueOnce(rowFixture('lib-b', 'Seriale'));
		fetchLibraryPopular.mockResolvedValueOnce(rowFixture('lib-b', 'Seriale'));
		fetchLibraryItems.mockResolvedValueOnce(itemsFixture('lib-b', 'Seriale'));

		// Navigate away to lib-b before lib-a's slow request ever resolved.
		flushSync(() => {
			mockParams.id = 'lib-b';
		});
		expect(await screen.findByRole('heading', { level: 1, name: 'Seriale' })).toBeTruthy();

		// lib-a's request finally resolves, late — it must be discarded,
		// not clobber lib-b's already-rendered state.
		resolveRecentA(rowFixture('lib-a', 'Filmy'));
		await Promise.resolve();
		await Promise.resolve();

		expect(screen.queryByRole('heading', { level: 1, name: 'Filmy' })).toBeNull();
		expect(screen.getByRole('heading', { level: 1, name: 'Seriale' })).toBeTruthy();
	});

	// Runda 11 — "All" section toolbar: Play All removed, everything else
	// (Shuffle, sort, pagination, letter row) intact, and the section is no
	// longer centered in a narrower column than the rows above it. This
	// project's tests run in the default locale (`en`, nothing here calls
	// `setLocale`) — same as every other test in this file — so the
	// assertions below match `en.json`, not `pl.json`.
	it('renders the "All" toolbar without Play All, with Shuffle/sort/pagination/letters intact and left-aligned', async () => {
		fetchLibraryRecent.mockResolvedValue(rowFixture('lib-a', 'Filmy'));
		fetchLibraryRecommended.mockResolvedValue(rowFixture('lib-a', 'Filmy'));
		fetchLibraryPopular.mockResolvedValue(rowFixture('lib-a', 'Filmy'));
		fetchLibraryItems.mockResolvedValue(nonEmptyItemsFixture('lib-a', 'Filmy'));

		const { container } = render(LibraryPage);

		// Wait for the real (non-skeleton) "All" grid to land — the counter
		// text below only exists once `total > 0` has resolved. Matched with
		// a regex rather than the literal en-dash character, so this isn't
		// fragile to exactly which dash glyph `en.json` happens to use.
		expect(await screen.findByText(/^1.1 of 1$/)).toBeTruthy();

		// The button itself is gone — not replaced by another disabled
		// button or left as dead markup.
		expect(screen.queryByText(/Play All/)).toBeNull();

		// Nothing else in the toolbar was accidentally removed alongside it.
		expect(screen.getByText(/Shuffle/)).toBeTruthy();
		expect(screen.getByLabelText('Previous page')).toBeTruthy();
		expect(screen.getByLabelText('Next page')).toBeTruthy();
		expect(screen.getByLabelText('Sort by')).toBeTruthy();
		expect(screen.getByText('Comfortable view')).toBeTruthy();
		expect(screen.getByText('Compact view')).toBeTruthy();
		expect(screen.getByText('All')).toBeTruthy(); // the letter-row's own "reset" button
		expect(screen.getByText('A')).toBeTruthy();
		expect(screen.getByText('Z')).toBeTruthy();

		// The regression guard for the alignment fix itself: the "All"
		// section's own wrapper no longer centers it in a `max-w-6xl` column
		// narrower than the `MediaRow` sections above it.
		const allSection = Array.from(container.querySelectorAll('section')).find((el) =>
			el.textContent?.includes('All — Filmy')
		);
		expect(allSection).toBeTruthy();
		expect(allSection?.className).not.toMatch(/\bmx-auto\b/);
		expect(allSection?.className).not.toMatch(/\bmax-w-/);
	});
});
