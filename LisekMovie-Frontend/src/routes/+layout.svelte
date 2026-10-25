<script lang="ts">
	import './layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import Navbar from '$lib/components/Navbar.svelte';
	import { page } from '$app/state';
	import { setLocale } from '$lib/i18n';
	import { initSpatialNavigation } from '$lib/spatial/spatial';
	import { watchInstall } from '$lib/pwa/install.svelte';
	import type { LayoutProps } from './$types';

	let { children, data }: LayoutProps = $props();

	// `+layout.ts` already resolved the right locale (account preference,
	// else the cookie the wizard set, else the default) into `data.locale`
	// — this just applies it to the reactive store every component's `t()`
	// reads from. Two calls, not one, because this root layout is never
	// recreated by SvelteKit's client-side router across navigations (only
	// `data` changes), so a plain top-level assignment alone would only
	// ever run once, on the very first mount:
	//   - the plain statement below covers the FIRST render of a given
	//     navigation, including the server-rendered HTML itself, so there's
	//     no flash of the wrong language before hydration. See the comment
	//     on `localeState` in `$lib/i18n/locale.svelte.ts` for why mutating
	//     it here, synchronously, is safe.
	//   - the `$effect` covers every render AFTER that — e.g. `data.locale`
	//     changing from "cookie only" to "the account's saved preference"
	//     right after signing in — without which the language could get
	//     stuck at whatever it resolved to on the very first page load,
	//     which was the original bug. Effects don't run during SSR, so this
	//     adds nothing there; the statement above already covers it.
	// The intentional-once-then-reactive read below is exactly what
	// `state_referenced_locally` warns about — this one is deliberate, see
	// the comment block above.
	// svelte-ignore state_referenced_locally
	setLocale(data.locale);
	$effect(() => {
		setLocale(data.locale);
	});

	// Arrow keys / TV remote D-pad move focus between cards, rows and
	// buttons on every page (see `$lib/spatial/spatial.ts`).
	$effect(() => initSpatialNavigation());

	// "Install app" in the account menu (PWA): keep the browser's install
	// prompt for when the user asks.
	$effect(() => watchInstall());

	// The player covers the whole screen; the navbar behind it would only
	// catch Tab focus.
	const watching = $derived(page.route.id?.startsWith('/watch/') ?? false);
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>

<!--
  The navbar only ever has a user to show once the Auth Guard (+layout.ts)
  has confirmed one — which also means it's naturally absent on /setup and
  /login, with no route-based check needed here.
-->
{#if data.user && !data.user.must_change_password && !watching}
	<Navbar username={data.user.username} isAdmin={data.user.is_admin} />
{/if}
{@render children()}
