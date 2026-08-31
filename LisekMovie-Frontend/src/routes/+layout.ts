// Runs before every navigation, on the server during SSR and again on the
// client for client-side navigations. Two independent redirect decisions
// live here, in priority order: (1) has the First-Run Wizard run yet, and
// (2) is this browser signed in — this is the ONE place both are decided,
// so no page needs to duplicate either check, and neither protected content
// nor the wizard/login forms ever flash before a redirect fires, since this
// runs (and can redirect) before any page component renders.
import { redirect } from '@sveltejs/kit';
import type { LayoutLoad } from './$types';
import { buildAuthHeader } from '$lib/auth';

export interface ServerStatus {
	setup_required: boolean;
	database_connected: boolean;
}

export interface AuthUser {
	id: string;
	username: string;
	is_admin: boolean;
	ui_language: string;
	email: string | null;
	created_at: string;
}

export const load: LayoutLoad = async ({ fetch, url, data }) => {
	let status: ServerStatus | null = null;

	try {
		const response = await fetch('/api/status');
		if (response.ok) {
			status = (await response.json()) as ServerStatus;
		}
	} catch {
		// Backend unreachable — e.g. the frontend dev server is running on
		// its own, with no `cargo run` behind the /api proxy yet. Render
		// normally rather than crashing the whole app on a network error;
		// `status` stays null and neither redirect below fires.
	}

	if (status?.setup_required && url.pathname !== '/setup') {
		redirect(307, '/setup');
	}

	if (status && !status.setup_required && url.pathname === '/setup') {
		redirect(307, '/');
	}

	// The sign-in check only makes sense once setup is done — before that
	// there's no admin account for anyone to be signed in as, and the
	// redirect above has already sent this request to /setup regardless.
	let user: AuthUser | null = null;
	if (status && !status.setup_required) {
		user = await fetchCurrentUser(fetch, data.token);

		if (!user && url.pathname !== '/login') {
			redirect(307, '/login');
		}
		if (user && url.pathname === '/login') {
			redirect(307, '/');
		}
	}

	return { status, user };
};

async function fetchCurrentUser(
	fetchFn: typeof fetch,
	token: string | null
): Promise<AuthUser | null> {
	if (!token) return null;

	try {
		const response = await fetchFn('/api/auth/me', { headers: buildAuthHeader(token) });
		if (!response.ok) return null;
		return (await response.json()) as AuthUser;
	} catch {
		return null;
	}
}
