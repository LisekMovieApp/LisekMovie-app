<!--
  Fixed top navigation. Transparent with a large logo over the hero banner
  at the top of the page; once the page scrolls, transitions to a fully
  solid bar (no blur, no partial opacity — a scrolled-past poster must not
  show through it) with a slightly smaller logo, both animated so neither
  change is a hard cut.

  Deliberately no border here, even a hairline one: toggling `border-b` on
  and off animates `border-bottom-width` from 0, and for the brief instant
  it's above 0 but the `border-white/5` color utility hasn't "caught up" in
  the same transition, the element renders Tailwind preflight's default
  border color (`currentColor` — white, on this light-text header) instead
  — a bright white flash right at the seam. Keeping the border out
  entirely, rather than trying to keep its width constant and animate only
  its color, is the simpler fix and matches what was actually asked for:
  only the background and the logo size are meant to animate here.

  "Moja Lista" and "Gry" are real, translated labels but not yet real
  routes: My List needs a per-account watch-history/favorites model that
  doesn't exist in the schema yet, and Gry is a native core feature planned
  for a later phase (see the roadmap). Shown dimmed and inert rather than
  linking to a page that doesn't exist yet.

  Runda 9: the admin's own Libraries (e.g. "Filmy", "Seriale") now render
  as real links between "Strona główna" and "Moja Lista" — fetched once,
  client-side, from `GET /api/libraries` (see `$lib/media/api`'s
  `fetchLibraries` doc comment: same shared `sort_order` the Admin Panel's
  reorder buttons control, so every signed-in account sees the identical
  order). `libraries` starts `[]` and a failed fetch just leaves it `[]` —
  deliberately non-fatal: a category list that never loaded shouldn't take
  the rest of the navbar (Home, the user menu) down with it, the same
  "degrade, don't break" idiom the Detail View's provider-order fetch
  already uses. Each link fades in with `svelte/transition`'s `fade` as it
  is added to the `{#each}` (the "menu appears smoothly once data has
  loaded" the request asked for) rather than the whole bar jumping the
  instant the response lands.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { fade } from 'svelte/transition';
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

	let scrolled = $state(false);
	let libraries: LibrarySummary[] = $state([]);

	onMount(() => {
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

		return () => window.removeEventListener('scroll', onScroll);
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

		<nav class="hidden items-center gap-4 text-sm sm:flex">
			<a href={resolve('/')} class="rounded-full bg-white/15 px-3 py-1 font-medium text-white">
				{t('nav.home')}
			</a>
			{#each libraries as library (library.id)}
				<a
					href={resolve('/library/[id]', { id: library.id })}
					class="px-1 text-neutral-300 transition-colors hover:text-white"
					transition:fade={{ duration: 200 }}
				>
					{library.name}
				</a>
			{/each}
			<span class="cursor-not-allowed px-1 text-neutral-500" title={t('nav.coming_soon')}>
				{t('nav.my_list')}
			</span>
			<span class="cursor-not-allowed px-1 text-neutral-500" title={t('nav.coming_soon')}>
				{t('nav.games')}
			</span>
		</nav>
	</div>

	<UserMenu {username} {isAdmin} />
</header>
