import { describe, it, expect } from 'vitest';
import { load } from './+layout';

function event(path: string, required: boolean) {
    return {
        url: new URL(`http://localhost${path}`),
        data: { token: 'test-token', localeFromCookie: 'pl' },
        fetch: async (url: string) => new Response(JSON.stringify(url === '/api/status'
            ? { setup_required: false, database_connected: true }
            : { id: 'test-user', username: 'test', is_admin: true, must_change_password: required, ui_language: 'pl' }),
            { status: 200, headers: { 'Content-Type': 'application/json' } })
    } as unknown as Parameters<typeof load>[0];
}
describe('mandatory password change', () => {
    it('redirects even administrators away from protected pages', async () => {
        await expect(load(event('/admin/users', true))).rejects.toMatchObject({ status: 307, location: '/change-password' });
    });
    it('does not loop on the password page', async () => {
        await expect(load(event('/change-password', true))).resolves.toMatchObject({ user: { must_change_password: true } });
    });
    it('allows navigation after the password is changed', async () => {
        await expect(load(event('/admin/users', false))).resolves.toMatchObject({ user: { must_change_password: false } });
    });
});
