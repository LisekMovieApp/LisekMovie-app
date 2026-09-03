// Runda 10 — test-only infrastructure. A minimal, mutable stand-in for
// `$app/state`'s reactive `page` object, used by this project's
// Svelte-component-level regression tests (see
// `src/routes/library/[id]/page.svelte.test.ts` and
// `src/routes/media/[id]/page.svelte.test.ts`) to prove the routing fix:
// that changing a route's `[id]` param — without unmounting the page —
// re-triggers that page's own `$effect`-driven data fetch.
//
// This file's `.svelte.ts` extension is what makes the `$state` rune legal
// here, outside an actual `.svelte` file. `createMockPageState` returns two
// references to the SAME reactive `params` object: `page.params` (what a
// test wires up via `vi.mock('$app/state', () => ({ page }))`, so the
// component under test reads it exactly like the real `$app/state`) and
// `params` itself (what the test mutates directly, e.g. `mockParams.id =
// 'lib-b'`). Because ES modules are singletons, both are the identical
// object — mutating one is observed through the other — so setting
// `mockParams.id` behaves exactly like a real client-side navigation to a
// new `/library/[id]` URL: any `$effect` that read `page.params.id`
// synchronously on its last run re-runs.
//
// Only the fields this app's pages actually read from `page` are
// implemented here — this is not a full `$app/state` shim.
import { SvelteURL } from 'svelte/reactivity';

export function createMockPageState(initialParams: Record<string, string>) {
	const params = $state({ ...initialParams });

	const page = {
		get params() {
			return params;
		},
		/** `page.data.user` — only `media/[id]/+page.svelte` reads this
		 * (`page.data.user?.is_admin`, to decide whether to fetch the
		 * admin-only provider list). `null` here means "not signed in as
		 * an admin", which is exactly what these tests want: they're
		 * proving the navigation/reactivity fix, not admin-only behavior. */
		data: { user: null as { is_admin: boolean } | null },
		// `SvelteURL` rather than the built-in `URL` — this app's ESLint
		// config (`svelte/prefer-svelte-reactivity`) requires it for any
		// mutable URL instance, even one like this that this mock never
		// actually mutates.
		url: new SvelteURL('http://localhost/'),
		route: { id: null as string | null },
		status: 200,
		error: null,
		form: null,
		state: {}
	};

	return { page, params };
}
