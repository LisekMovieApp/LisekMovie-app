// Faza 19 — the account menu: who is signed in and with which role, the
// profile link, the notification switch for this device, the admin panel
// only for administrators, and Log out at the very bottom.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, within } from '@testing-library/svelte';
import UserMenu from './UserMenu.svelte';

vi.mock('$app/state', async () => {
	const { createMockPageState } = await import('$lib/testing/mockPageState.svelte');
	const { page } = createMockPageState({});
	return { page };
});
vi.mock('$app/paths', () => ({ resolve: (route: string) => route, base: '', assets: '' }));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$lib/auth', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/auth')>();
	return { ...actual, getSessionCookieClientSide: () => 'tok' };
});

beforeEach(() => {
	localStorage.clear();
});

function menu() {
	return screen.getByRole('button', { name: 'Account menu' });
}

describe('UserMenu', () => {
	it('shows the user, the role and the actions in order', async () => {
		render(UserMenu, { username: 'lemon', isAdmin: false });
		await fireEvent.click(menu());
		const panel = menu().nextElementSibling as HTMLElement;
		expect(panel.hidden).toBe(false);
		expect(within(panel).getByText('lemon').tagName).toBe('STRONG');
		expect(within(panel).getByText(/Signed in as:/)).toBeTruthy();
		expect(within(panel).getByText('Role: User')).toBeTruthy();
		expect(
			within(panel)
				.getByRole('link', { name: /Profile settings/ })
				.getAttribute('href')
		).toBe('/settings/profile');
		expect(within(panel).queryByRole('link', { name: /Admin Panel/ })).toBeNull();
		// Notifications are off on this device, so the switch offers turning them on.
		expect(within(panel).getByRole('button', { name: /Notifications: Turn on/ })).toBeTruthy();
		const buttons = within(panel).getAllByRole('button');
		expect(buttons.at(-1)?.textContent).toMatch(/Log out/);
	});

	it('offers the admin panel to administrators', async () => {
		render(UserMenu, { username: 'admin', isAdmin: true });
		await fireEvent.click(menu());
		expect(screen.getByText('Role: Administrator')).toBeTruthy();
		expect(screen.getByRole('link', { name: /Admin Panel/ })).toBeTruthy();
	});

	it('shows the avatar picture, or the initial without one', async () => {
		const { unmount } = render(UserMenu, {
			username: 'lemon',
			isAdmin: false,
			avatarUrl: '/api/users/1/avatar?v=2'
		});
		const img = menu().querySelector('img');
		expect(img?.getAttribute('src')).toBe('/api/users/1/avatar?v=2&token=tok');
		unmount();
		render(UserMenu, { username: 'lemon', isAdmin: false });
		expect(menu().querySelector('img')).toBeNull();
		expect(menu().textContent?.trim()).toBe('L');
	});

	it('closes on Escape', async () => {
		render(UserMenu, { username: 'lemon', isAdmin: false });
		await fireEvent.click(menu());
		await fireEvent.keyDown(window, { key: 'Escape' });
		expect((menu().nextElementSibling as HTMLElement).hidden).toBe(true);
		expect(menu().getAttribute('aria-expanded')).toBe('false');
	});
});
