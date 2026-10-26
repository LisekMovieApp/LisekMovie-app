// Runda 10 — the identical navigation/reactivity fix applied to
// `/library/[id]` was applied here too (same latent bug, found during that
// round's audit: `onMount(load)` never re-fires when SvelteKit reuses this
// component instance across a `/media/[id]` → `/media/[id]` navigation that
// only changes the `id` param). This mirrors
// `src/routes/library/[id]/page.svelte.test.ts`'s own structure and mocking
// strategy — see that file's top comment for why the `$app/state` mock is
// built via a dynamic `import()` inside the (hoisted) `vi.mock` factory
// instead of a same-file top-level `const` (Vitest hoists `vi.mock` above
// the file's own imports, so a same-file `const` closed over there is still
// in its temporal dead zone when the factory first runs).
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { flushSync } from 'svelte';
import { render, screen, waitFor } from '@testing-library/svelte';
import MediaPage from './+page.svelte';
import { page as mockPage } from '$app/state';
import type { MediaItem } from '$lib/media/api';

vi.mock('$app/state', async () => {
	const { createMockPageState } = await import('$lib/testing/mockPageState.svelte');
	const { page } = createMockPageState({ id: 'item-a' });
	return { page };
});

vi.mock('$app/paths', () => ({
	resolve: (route: string, params?: Record<string, string>) =>
		route.replace(/\[(\w+)\]/g, (_match, key: string) => params?.[key] ?? ''),
	base: '',
	assets: ''
}));

const { fetchMediaDetail } = vi.hoisted(() => ({
	fetchMediaDetail: vi.fn()
}));

vi.mock('$lib/media/api', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/media/api')>();
	return {
		...actual,
		fetchMediaDetail,
		// Not under test here — an admin-only, always-non-fatal secondary
		// fetch this page only makes for a 'series' item; every fixture
		// below is a 'movie', so this is never even called.
		fetchSeriesSeasons: vi.fn().mockResolvedValue([])
	};
});

// `page.params` is the same reactive object every time this getter is read
// (see `mockPageState.svelte`'s own comment) — mutating it through this
// alias is exactly how these tests simulate a client-side navigation.
const mockParams = mockPage.params as { id: string };

function itemFixture(id: string, title: string): MediaItem {
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

describe('/media/[id] page', () => {
	beforeEach(() => {
		fetchMediaDetail.mockReset();
		document.cookie = 'lisek_session=test-token; Path=/';
		mockParams.id = 'item-a';
	});

	it('fetches the item present at mount', async () => {
		fetchMediaDetail.mockResolvedValue(itemFixture('item-a', 'Test Movie A'));

		render(MediaPage);

		await waitFor(() => expect(fetchMediaDetail).toHaveBeenCalledWith('test-token', 'item-a'));
		expect(await screen.findByRole('heading', { level: 1, name: 'Test Movie A' })).toBeTruthy();
	});

	it('re-fetches and clears stale content when the id changes, without unmounting the page', async () => {
		fetchMediaDetail.mockResolvedValueOnce(itemFixture('item-a', 'Test Movie A'));

		render(MediaPage);
		expect(await screen.findByRole('heading', { level: 1, name: 'Test Movie A' })).toBeTruthy();

		let resolveB: (value: MediaItem) => void = () => {};
		fetchMediaDetail.mockReturnValueOnce(new Promise((resolve) => (resolveB = resolve)));

		// Simulates following a link to a different item without a full
		// page reload — this component instance is never unmounted here.
		flushSync(() => {
			mockParams.id = 'item-b';
		});

		expect(fetchMediaDetail).toHaveBeenCalledWith('test-token', 'item-b');
		// The core assertion: item A's title is gone from the DOM the
		// instant the id changed — before item B's request has resolved.
		expect(screen.queryByRole('heading', { level: 1, name: 'Test Movie A' })).toBeNull();

		resolveB(itemFixture('item-b', 'Test Movie B'));
		expect(await screen.findByRole('heading', { level: 1, name: 'Test Movie B' })).toBeTruthy();
	});

	it('discards a stale response from an id that is no longer current', async () => {
		let resolveA: (value: MediaItem) => void = () => {};
		fetchMediaDetail.mockReturnValueOnce(new Promise((resolve) => (resolveA = resolve)));

		render(MediaPage);

		fetchMediaDetail.mockResolvedValueOnce(itemFixture('item-b', 'Test Movie B'));
		flushSync(() => {
			mockParams.id = 'item-b';
		});
		expect(await screen.findByRole('heading', { level: 1, name: 'Test Movie B' })).toBeTruthy();

		// item A's request finally resolves, late — it must be discarded.
		resolveA(itemFixture('item-a', 'Test Movie A'));
		await Promise.resolve();
		await Promise.resolve();

		expect(screen.queryByRole('heading', { level: 1, name: 'Test Movie A' })).toBeNull();
		expect(screen.getByRole('heading', { level: 1, name: 'Test Movie B' })).toBeTruthy();
	});
});
