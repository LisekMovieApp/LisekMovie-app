<!--
  Top navigation (Faza 4, VOD style): transparent over the hero with a
  dark gradient for legibility, solid and blurred once the page scrolls.
  The current section is marked with `aria-current="page"` (white text and
  a gold underline). Below `md` the links move into a slide-in drawer
  behind a menu button, so they stay large enough to tap. Library links
  come from `fetchLibraries`; a failed fetch just leaves them out
  (non-fatal — Home and My List still work).
-->
<script lang="ts">
	import { api } from '$lib/playback/api';
	import { onMount } from 'svelte';
	import { fade, fly } from 'svelte/transition';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import type { ResolvedPathname } from '$app/types';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchLibraries, type LibrarySummary } from '$lib/media/api';
	import { navScope } from '$lib/spatial/spatial';
	import logoText from '$lib/assets/logo_text.svg';
	import Icon from './Icon.svelte';
	import UserMenu from './UserMenu.svelte';
	import SearchOverlay from './SearchOverlay.svelte';
	import { isSearchShortcut } from '$lib/media/search';
	import { startHeartbeat } from '$lib/users/profile';

	interface Props {
		username: string;
		isAdmin: boolean;
		avatarUrl?: string | null;
	}

	let { username, isAdmin, avatarUrl = null }: Props = $props();

	let gamesEnabled = $state(false);
	let scrolled = $state(false);
	let drawer = $state(false);
	let libraries: LibrarySummary[] = $state([]);
	/** Faza 11: the omnibox layer, and the button that gets focus back. */
	let searching = $state(false);
	let searchButton: HTMLButtonElement | undefined = $state();
	const searchToken = $derived(searching ? (getSessionCookieClientSide() ?? '') : '');

	function closeSearch() {
		if (!searching) return;
		searching = false;
		queueMicrotask(() => searchButton?.focus({ preventScroll: true }));
	}

	function onShortcut(event: KeyboardEvent) {
		if (searching || !isSearchShortcut(event)) return;
		event.preventDefault();
		drawer = false;
		searching = true;
	}

	/** Shared by every nav link: active and inactive share the same sizing
	 * so a link doesn't change shape when it turns on or off, only its
	 * colour and the underline (`aria-current` variants) do. */
	const NAV_ITEM =
		'relative rounded-md px-3 py-2 font-medium text-neutral-300 transition-colors [text-shadow:0_1px_3px_rgb(0_0_0/0.7)] after:absolute after:inset-x-3 after:bottom-0.5 after:h-0.5 after:origin-left after:scale-x-0 after:rounded-full after:bg-gold after:transition-transform hover:text-white focus-visible:bg-white/10 focus-visible:text-white aria-[current=page]:text-white aria-[current=page]:after:scale-x-100';
	const DRAWER_ITEM =
		'flex items-center rounded-lg px-4 py-3 text-lg font-medium text-neutral-300 hover:bg-white/10 hover:text-white focus-visible:bg-white/10 aria-[current=page]:bg-white/10 aria-[current=page]:text-white aria-[current=page]:shadow-[inset_3px_0_0_var(--color-gold)]';

	interface NavLink {
		key: string;
		href: ResolvedPathname;
		label: string;
		/** Sections with sub-pages count as current anywhere below them. */
		prefix?: boolean;
	}

	const links = $derived.by((): NavLink[] => [
		{ key: 'home', href: resolve('/'), label: t('nav.home') },
		...libraries.map((library) => ({
			key: library.id,
			href: resolve('/library/[id]', { id: library.id }),
			label: library.name
		})),
		{ key: 'my-list', href: resolve('/my-list'), label: t('nav.my_list'), prefix: true },
		...(gamesEnabled
			? [{ key: 'games', href: resolve('/games'), label: t('nav.games'), prefix: true }]
			: [])
	]);

	/** Matched by exact path (libraries by id, never by name), or below a prefix section. */
	function current(link: NavLink): 'page' | undefined {
		const path = page.url.pathname;
		return path === link.href || (link.prefix && path.startsWith(`${link.href}/`))
			? 'page'
			: undefined;
	}

	$effect(() => {
		// Any navigation closes the drawer.
		void page.url.pathname;
		drawer = false;
	});

	$effect(() => {
		if (!drawer) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key === 'Escape') drawer = false;
		};
		const onBack = (event: Event) => {
			event.preventDefault();
			drawer = false;
		};
		window.addEventListener('keydown', onKey);
		window.addEventListener('spatialback', onBack);
		return () => {
			window.removeEventListener('keydown', onKey);
			window.removeEventListener('spatialback', onBack);
		};
	});

	onMount(() => {
		let active = true;
		const refreshGames = () => {
			const token = getSessionCookieClientSide();
			if (token)
				api<unknown[]>(token, '/api/games')
					.then((games) => {
						if (active) gamesEnabled = games.length > 0;
					})
					.catch(() => {});
		};
		refreshGames();
		window.addEventListener('lisek-games-changed', refreshGames);
		window.addEventListener('focus', refreshGames);

		const onScroll = () => {
			scrolled = window.scrollY > 24;
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

		// Faza 19: online status in Admin -> Users while this tab is visible.
		const stopHeartbeat = startHeartbeat(getSessionCookieClientSide);

		return () => {
			active = false;
			stopHeartbeat();
			window.removeEventListener('scroll', onScroll);
			window.removeEventListener('lisek-games-changed', refreshGames);
			window.removeEventListener('focus', refreshGames);
		};
	});
</script>

<header
	class="fixed inset-x-0 top-0 z-50 flex h-16 items-center justify-between gap-4 px-4 transition-[background-color,box-shadow,backdrop-filter] duration-300 md:h-[4.5rem] md:px-8 {scrolled
		? 'bg-black/85 shadow-[0_6px_24px_rgb(0_0_0/0.5)] backdrop-blur-md'
		: 'bg-gradient-to-b from-black/80 via-black/40 to-transparent'}"
>
	<div class="flex min-w-0 items-center gap-2 md:gap-6">
		<button
			type="button"
			class="grid size-10 shrink-0 place-items-center rounded-md text-white hover:bg-white/10 md:hidden"
			aria-label={t('nav.menu')}
			aria-expanded={drawer}
			onclick={() => (drawer = !drawer)}
		>
			<Icon name="menu" class="size-6" />
		</button>
		<a href={resolve('/')} class="shrink-0 rounded-md">
			<img src={logoText} alt="LisekMovie" class="h-9 w-auto md:h-11" />
		</a>

		<nav
			class="hidden min-w-0 [scrollbar-width:none] items-center gap-1 overflow-x-auto text-sm whitespace-nowrap md:flex lg:text-[0.95rem]"
			aria-label={t('nav.main')}
		>
			{#each links as link (link.key)}
				<a
					href={link.href}
					aria-current={current(link)}
					class={NAV_ITEM}
					in:fade={{ duration: 200 }}>{link.label}</a
				>
			{/each}
		</nav>
	</div>

	<div class="flex shrink-0 items-center gap-1 md:gap-2">
		<button
			bind:this={searchButton}
			type="button"
			class="grid size-10 place-items-center rounded-full text-white transition-colors hover:bg-white/10 hover:text-gold focus-visible:bg-white/15 focus-visible:text-gold md:size-11"
			aria-label={t('search.open')}
			title={t('search.open_hint')}
			aria-haspopup="dialog"
			aria-expanded={searching}
			onclick={() => (searching = true)}
		>
			<Icon name="search" class="size-6" />
		</button>
		<UserMenu {username} {isAdmin} {avatarUrl} />
	</div>
</header>

<svelte:window onkeydown={onShortcut} />

{#if searching && searchToken}
	<SearchOverlay token={searchToken} onclose={closeSearch} />
{/if}

{#if drawer}
	<button
		type="button"
		class="fixed inset-0 z-50 bg-black/60 backdrop-blur-sm md:hidden"
		aria-label={t('nav.close_menu')}
		tabindex="-1"
		data-nav-skip
		onclick={() => (drawer = false)}
		transition:fade={{ duration: 150 }}
	></button>
	<nav
		class="fixed inset-y-0 left-0 z-50 flex w-72 max-w-[85vw] flex-col gap-1 overflow-y-auto bg-neutral-950 p-4 pt-5 shadow-2xl ring-1 ring-white/10 md:hidden"
		aria-label={t('nav.main')}
		use:navScope={{ autofocus: true }}
		transition:fly={{ x: -288, duration: 220 }}
	>
		<div class="mb-4 flex items-center justify-between">
			<img src={logoText} alt="LisekMovie" class="h-9 w-auto" />
			<button
				type="button"
				class="grid size-10 place-items-center rounded-md text-white hover:bg-white/10"
				aria-label={t('nav.close_menu')}
				onclick={() => (drawer = false)}
			>
				<Icon name="close" class="size-6" />
			</button>
		</div>
		{#each links as link (link.key)}
			<a
				href={link.href}
				aria-current={current(link)}
				class={DRAWER_ITEM}
				data-nav-default={current(link) ? '' : undefined}>{link.label}</a
			>
		{/each}
	</nav>
{/if}
