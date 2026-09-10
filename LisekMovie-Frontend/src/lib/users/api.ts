export interface Account {
    id: string; username: string; is_admin: boolean; disabled: boolean;
    must_change_password: boolean; ui_language: string;
}
import { buildAuthHeader } from '$lib/auth';
export async function accountRequest<T>(token: string, path: string, method = 'GET', body?: unknown): Promise<T> {
    const response = await fetch(path, { method, cache: 'no-store',
        headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }) });
    const data = await response.json().catch(() => null);
    if (!response.ok) throw new Error(data?.error_code ?? 'internal_error');
    return data as T;
}
/** Retain error codes so a visible error follows subsequent locale changes. */
export function accountErrorKey(error: unknown): string {
    const known = new Set(["username_taken", "invalid_user", "invalid_password", "last_administrator", "password_must_differ", "incorrect_current_password", "credentials_changed", "invalid_token", "admin_required", "password_change_required"]);
    const code = error instanceof Error ? error.message : '';
    return `accounts.errors.${known.has(code) ? code : 'operation_failed'}`;
}
