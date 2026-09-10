import { describe, it, expect, vi, afterEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { setLocale } from '$lib/i18n';
import Page from './+page.svelte';

vi.mock('$app/state', () => ({ page: { data: { user: { must_change_password: true } } } }));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

afterEach(() => setLocale('en'));
describe('password form localization', () => {
    it('updates labels and an existing validation error when the locale changes', async () => {
        setLocale('pl');
        const { container } = render(Page);
        expect(screen.getByRole('heading', { name: 'Ustaw własne hasło' })).toBeTruthy();
        await fireEvent.input(screen.getByLabelText('Nowe hasło'), { target: { value: 'password-one' } });
        await fireEvent.input(screen.getByLabelText('Powtórz nowe hasło'), { target: { value: 'password-two' } });
        await fireEvent.submit(container.querySelector('form')!);
        expect(screen.getByRole('alert').textContent).toBe('Hasła nie są takie same.');
        flushSync(() => setLocale('en'));
        expect(screen.getByRole('heading', { name: 'Set your own password' })).toBeTruthy();
        expect(screen.getByLabelText('New password')).toBeTruthy();
        expect(screen.getByRole('alert').textContent).toBe('The passwords do not match.');
    });
});
