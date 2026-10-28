// Faza 19 — Admin → Users: avatars, online dots and the debounced name filter.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import Page from './+page.svelte';

const { accountRequest } = vi.hoisted(() => ({ accountRequest: vi.fn() }));

vi.mock('$app/state', async () => {
	const { createMockPageState } = await import('$lib/testing/mockPageState.svelte');
	const { page } = createMockPageState({});
	return { page };
});
vi.mock('$app/paths', () => ({ resolve: (route: string) => route, base: '', assets: '' }));
vi.mock('$app/navigation', () => ({ goto: vi.fn(), invalidateAll: vi.fn() }));
vi.mock('$lib/auth', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/auth')>();
	return { ...actual, getSessionCookieClientSide: () => 'tok' };
});
vi.mock('$lib/users/api', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/users/api')>();
	return { ...actual, accountRequest };
});
vi.mock('$lib/users/restrictions', () => ({ listRestricted: vi.fn().mockResolvedValue([]) }));
vi.mock('$lib/admin/libraries', () => ({ fetchLibraries: vi.fn().mockResolvedValue([]) }));

const account = (id: string, username: string, extra: object = {}) => ({
	id,
	username,
	is_admin: false,
	disabled: false,
	must_change_password: false,
	ui_language: 'en',
	avatar_url: null,
	online: false,
	last_seen_at: null,
	now_playing: null,
	...extra
});

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
	accountRequest.mockReset();
	accountRequest.mockResolvedValue([
		account('1', 'Anna', { online: true, avatar_url: '/api/users/1/avatar?v=1', is_admin: true }),
		account('2', 'Bartek', { online: true, now_playing: 'Rojbers' }),
		account('3', 'Celina', { last_seen_at: new Date(Date.now() - 5 * 60_000).toISOString() })
	]);
});

describe('Admin users', () => {
	it('shows avatars and who is online', async () => {
		render(Page);
		await wait(0);
		expect(await screen.findByText('Anna')).toBeTruthy();
		expect(screen.getByText('2 of 3 online')).toBeTruthy();
		expect(screen.getAllByRole('img', { name: 'Online' })).toHaveLength(2);
		expect(screen.getAllByRole('img', { name: 'Offline' })).toHaveLength(1);
		expect(screen.getByText('Watching: Rojbers')).toBeTruthy();
		expect(screen.getByText('Last seen 5 minutes ago')).toBeTruthy();
		expect(document.querySelector('img[src="/api/users/1/avatar?v=1&token=tok"]')).toBeTruthy();
	});

	it('filters by name after a short pause', async () => {
		render(Page);
		await screen.findByText('Anna');
		const search = screen.getByRole('searchbox', { name: 'Search users…' });
		await fireEvent.input(search, { target: { value: 'bar' } });
		// Not yet: the filter waits for typing to pause.
		expect(screen.queryByText('Anna')).toBeTruthy();
		await wait(250);
		expect(screen.queryByText('Anna')).toBeNull();
		expect(screen.getByText('Bartek')).toBeTruthy();
		await fireEvent.input(search, { target: { value: 'zzz' } });
		await wait(250);
		expect(screen.getByText('No user matches this name.')).toBeTruthy();
	});
});
