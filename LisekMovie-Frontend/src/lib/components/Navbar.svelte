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
  routes: My List needs the library scanner's data (Faza 5+) and Gry is a
  native core feature planned for a later phase (see the roadmap). Shown
  dimmed and inert rather than linking to a page that doesn't exist yet.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import logoText from '$lib/assets/logo_text.svg';
	import UserMenu from './UserMenu.svelte';

	interface Props {
		username: string;
		isAdmin: boolean;
	}

	let { username, isAdmin }: Props = $props();

	let scrolled = $state(false);

	onMount(() => {
		const onScroll = () => {
			scrolled = window.scrollY > 40;
		};
		onScroll();
		window.addEventListener('scroll', onScroll, { passive: true });
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
