<script lang="ts">
	import { api } from '$lib/playback/api';
	import { onMount } from 'svelte';
	import { fade } from 'svelte/transition';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchLibraries, type LibrarySummary } from '$lib/media/api';
	import logoText from '$lib/assets/logo_text.svg';
	import UserMenu from './UserMenu.svelte';

	interface Props {
		username: string;
		isAdmin: boolean;
	}

	let { username, isAdmin }: Props = $props();

	let gamesEnabled = $state(false);
	let scrolled = $state(false);
	let libraries: LibrarySummary[] = $state([]);

	/** Shared by both Home and every Library link below — active and
	 * inactive share the same `rounded-full px-3 py-1 font-medium` sizing
	 * so the pill doesn't change shape/width when it turns on or off, only
	 * `bg`/`text` do. The active half is bit-identical to this navbar's
	 * original, hardcoded Home style. */
	const NAV_ITEM_BASE = 'rounded-full px-3 py-1 font-medium transition-colors';
	const NAV_ITEM_ACTIVE = 'bg-white/15 text-white';
	const NAV_ITEM_INACTIVE = 'text-neutral-300 hover:text-white';

	/** Home is the one nav item that isn't inside an `{#each}` — `{@const}`
	 * is only legal as the direct child of a block (`{#each}`, `{#if}`,
	 * etc.), not of a plain element like `<nav>`, so its own active check
	 * lives here as a `$derived` instead. */
	const homeHref = resolve('/');
	let homeActive = $derived(page.url.pathname === homeHref);

	onMount(() => {
        let active = true;
        const refreshGames = () => {
            const token = getSessionCookieClientSide();
            if (token) api<unknown[]>(token, '/api/games').then(games => { if (active) gamesEnabled = games.length > 0; }).catch(() => {});
        };
        refreshGames();
        window.addEventListener('lisek-games-changed', refreshGames);
        window.addEventListener('focus', refreshGames);

		const onScroll = () => {
			scrolled = window.scrollY > 40;
		};
		onScroll();
		window.addEventListener('scroll', onScroll, { passive: true });

		const token = getSessionCookieClientSide();
		if (token) {
			fetchLibraries(token)
				.then((fetched) => (libraries = fetched))
				.catch(() => {
					// Non-fatal — see this component's own top comment.
				});
		}

		return () => { active = false; window.removeEventListener('scroll', onScroll); window.removeEventListener('lisek-games-changed', refreshGames); window.removeEventListener('focus', refreshGames); };
	});
</script>

<header
	class="fixed inset-x-0 top-0 z-50 flex items-center justify-between px-4 py-3 transition-all duration-300 md:px-8 {scrolled
		? 'bg-base'
		: 'bg-transparent'}"
>
	<div class="flex items-center gap-6">
		<a href={resolve('/')}>
			<img
				src={logoText}
				alt="LisekMovie"
				class="transition-all duration-300 {scrolled ? 'h-13' : 'h-16'}"
			/>
		</a>

		<nav class="flex max-w-[65vw] items-center gap-2 overflow-x-auto whitespace-nowrap text-sm">
			<a href={homeHref} class="{NAV_ITEM_BASE} {homeActive ? NAV_ITEM_ACTIVE : NAV_ITEM_INACTIVE}">
				{t('nav.home')}
			</a>
			{#each libraries as library (library.id)}
				{@const libraryHref = resolve('/library/[id]', { id: library.id })}
				{@const libraryActive = page.url.pathname === libraryHref}
				<a
					href={libraryHref}
					class="{NAV_ITEM_BASE} {libraryActive ? NAV_ITEM_ACTIVE : NAV_ITEM_INACTIVE}"
					transition:fade={{ duration: 200 }}
				>
					{library.name}
				</a>
			{/each}
			<a href={resolve('/my-list')} class={NAV_ITEM_BASE}>
				{t('nav.my_list')}
			</a>
			{#if gamesEnabled}<a href={resolve('/games')} class={NAV_ITEM_BASE}>{t('nav.games')}</a>{/if}
		</nav>
	</div>

	<UserMenu {username} {isAdmin} />
</header>
