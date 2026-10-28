// Faza 19 — Profile settings: avatar actions, password confirmation and the
// note about the server's default language.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import Page from './+page.svelte';
import { page as mockPage } from '$app/state';

const { setLanguage, deleteAvatar, invalidateAll } = vi.hoisted(() => ({
	setLanguage: vi.fn(),
	deleteAvatar: vi.fn(),
	invalidateAll: vi.fn()
}));

vi.mock('$app/state', async () => {
	const { createMockPageState } = await import('$lib/testing/mockPageState.svelte');
	const { page } = createMockPageState({});
	return { page };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn(), invalidateAll }));
vi.mock('$lib/auth', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/auth')>();
	return { ...actual, getSessionCookieClientSide: () => 'tok' };
});
vi.mock('$lib/users/profile', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/users/profile')>();
	return { ...actual, setLanguage, deleteAvatar };
});

function signIn(extra: object = {}) {
	(mockPage.data as { user: unknown }).user = {
		id: 'u1',
		username: 'lemon',
		is_admin: false,
		must_change_password: false,
		disabled: false,
		ui_language: 'en',
		email: null,
		created_at: '2026-01-01T00:00:00Z',
		avatar_url: null,
		server_language: 'pl',
		...extra
	};
}

beforeEach(() => {
	setLanguage.mockReset().mockResolvedValue(undefined);
	deleteAvatar.mockReset().mockResolvedValue(undefined);
	invalidateAll.mockReset().mockResolvedValue(undefined);
});

describe('Profile settings', () => {
	it('explains that the library stays in the server language', () => {
		signIn();
		render(Page);
		expect(screen.getByRole('note').textContent).toContain(
			"The server's default language is: Polski."
		);
	});

	it('removes the picture only when there is one', async () => {
		signIn();
		const first = render(Page);
		expect(
			(screen.getByRole('button', { name: /Remove picture/ }) as HTMLButtonElement).disabled
		).toBe(true);
		first.unmount();
		signIn({ avatar_url: '/api/users/u1/avatar?v=3' });
		render(Page);
		const [remove] = screen.getAllByRole('button', { name: /Remove picture/ });
		await fireEvent.click(remove);
		expect(deleteAvatar).toHaveBeenCalledWith('tok');
		expect(invalidateAll).toHaveBeenCalled();
	});

	it('blocks saving while the new passwords differ', async () => {
		signIn();
		render(Page);
		await fireEvent.input(screen.getByLabelText('New password'), {
			target: { value: 'abcdefgh1' }
		});
		await fireEvent.input(screen.getByLabelText('Confirm new password'), {
			target: { value: 'abcdefgh2' }
		});
		const save = screen.getByRole('button', { name: 'Save new password' }) as HTMLButtonElement;
		expect(save.disabled).toBe(true);
		expect(screen.getByText('The passwords do not match.')).toBeTruthy();
	});

	it('saves the interface language', async () => {
		signIn();
		render(Page);
		await fireEvent.change(screen.getByLabelText('Interface language'), {
			target: { value: 'pl' }
		});
		expect(setLanguage).toHaveBeenCalledWith('tok', 'pl');
	});
});
