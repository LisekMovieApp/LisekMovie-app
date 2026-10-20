// Runda 11 — Navbar's active-item highlighting: proves the same invariant
// Runda 10's own regression tests proved for the pages themselves (see
// `src/routes/library/[id]/page.svelte.test.ts`'s own top comment for why
// `$app/state` is mocked via a dynamic `import()` inside `vi.mock`'s
// factory, and `vi.hoisted` for the plain `vi.fn()` mocks) — that switching
// the current route (Home → a Library → another Library) without
// unmounting anything updates which nav item is highlighted, always
// exactly one at a time, matched by library `id` (never by name or a
// shared "library" substring), and that this happens without a reload.
// Unlike the page-level tests, Navbar's own active-state read has no
// `onMount` to get wrong in the first place (it's a plain reactive read of
// `page.url.pathname` in the markup) — these tests are the proof that
// holds, not a fix being verified.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { flushSync } from 'svelte';
import { render, screen } from '@testing-library/svelte';
import Navbar from './Navbar.svelte';
import { page as mockPage } from '$app/state';
import type { LibrarySummary } from '$lib/media/api';

vi.mock('$app/state', async () => {
	const { createMockPageState } = await import('$lib/testing/mockPageState.svelte');
	const { page } = createMockPageState({});
	return { page };
});

vi.mock('$app/paths', () => ({
	resolve: (route: string, params?: Record<string, string>) =>
		route.replace(/\[(\w+)\]/g, (_match, key: string) => params?.[key] ?? ''),
	base: '',
	assets: ''
}));

// Navbar renders `UserMenu`, whose sign-out button calls `goto` — never
// clicked by these tests, but mocked defensively so importing it can never
// depend on a real SvelteKit navigation context existing in jsdom.
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));

const { fetchLibraries } = vi.hoisted(() => ({
	fetchLibraries: vi.fn()
}));

vi.mock('$lib/media/api', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/media/api')>();
	return { ...actual, fetchLibraries };
});

function librariesFixture(): LibrarySummary[] {
	return [
		{ id: 'lib-films', name: 'Filmy', library_type: 'movies' },
		{ id: 'lib-series', name: 'Seriale', library_type: 'tv_shows' }
	];
}

/** Active items carry `aria-current="page"` (Faza 4: the white text and
 * gold underline are `aria-current` variants, see `Navbar.svelte`'s own
 * `NAV_ITEM`); inactive ones never do. An attribute check, not a
 * computed-style one, matching how this project's other component tests
 * assert on rendered markup. */
function isActive(el: Element | null): boolean {
	return !!el && el.getAttribute('aria-current') === 'page';
}

describe('Navbar active item', () => {
	beforeEach(() => {
		fetchLibraries.mockReset();
		document.cookie = 'lisek_session=test-token; Path=/';
		mockPage.url.pathname = '/';
	});

	it('highlights only "Home" on /', async () => {
		fetchLibraries.mockResolvedValue(librariesFixture());

		render(Navbar, { props: { username: 'lemon', isAdmin: false } });

		const home = await screen.findByText('Home');
		const filmy = await screen.findByText('Filmy');
		const seriale = screen.getByText('Seriale');
		expect(isActive(home)).toBe(true);
		expect(isActive(filmy)).toBe(false);
		expect(isActive(seriale)).toBe(false);
	});

	it('highlights only the current library, matched by id — never by name or by both libraries at once', async () => {
		fetchLibraries.mockResolvedValue(librariesFixture());
		mockPage.url.pathname = '/library/lib-series';

		render(Navbar, { props: { username: 'lemon', isAdmin: false } });

		const seriale = await screen.findByText('Seriale');
		const filmy = screen.getByText('Filmy');
		const home = screen.getByText('Home');
		expect(isActive(seriale)).toBe(true);
		expect(isActive(filmy)).toBe(false);
		expect(isActive(home)).toBe(false);
	});

	it('moves the highlight across Filmy → Seriale → Filmy without unmounting, exactly one item active at a time', async () => {
		fetchLibraries.mockResolvedValue(librariesFixture());
		mockPage.url.pathname = '/library/lib-films';

		render(Navbar, { props: { username: 'lemon', isAdmin: false } });
		await screen.findByText('Filmy');

		function activeCount(): number {
			return [
				screen.getByText('Home'),
				screen.getByText('Filmy'),
				screen.getByText('Seriale')
			].filter(isActive).length;
		}

		expect(isActive(screen.getByText('Filmy'))).toBe(true);
		expect(activeCount()).toBe(1);

		// Simulates clicking "Seriale" in this very navbar — a client-side
		// navigation to a new `/library/[id]` URL. Navbar is never
		// unmounted across this (it lives in the root layout), so this is
		// exactly the reactive read this round relies on, not a remount.
		flushSync(() => {
			mockPage.url.pathname = '/library/lib-series';
		});
		expect(isActive(screen.getByText('Seriale'))).toBe(true);
		expect(isActive(screen.getByText('Filmy'))).toBe(false);
		expect(activeCount()).toBe(1);

		flushSync(() => {
			mockPage.url.pathname = '/library/lib-films';
		});
		expect(isActive(screen.getByText('Filmy'))).toBe(true);
		expect(isActive(screen.getByText('Seriale'))).toBe(false);
		expect(activeCount()).toBe(1);
	});
});
