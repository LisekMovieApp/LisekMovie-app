// Route guard for everything under /admin/**: only a signed-in admin
// account may render anything here. The root layout (`+layout.ts`) already
// handles "signed in at all" (redirecting to /login otherwise) — this adds
// the stricter "and also an admin" check on top of that, redirecting a
// signed-in non-admin back to the home page rather than exposing even the
// empty shell of an admin page to them. Mirrors the root layout's own
// plain-string `redirect(307, '/...')` style rather than introducing a new
// convention here.
import { redirect } from '@sveltejs/kit';
import type { LayoutLoad } from './$types';

export const load: LayoutLoad = async ({ parent }) => {
	const { user } = await parent();

	if (!user?.is_admin) {
		redirect(307, '/');
	}
};
