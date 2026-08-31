<!--
  Fixed top navigation. Transparent with a large logo over the hero banner
  at the top of the page; once the page scrolls, transitions to a fully
  solid bar (no blur, no partial opacity — a scrolled-past poster must not
  show through it) with a slightly smaller logo, both animated so neither
  change is a hard cut.

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
	}

	let { username }: Props = $props();

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
		? 'border-b border-white/5 bg-base'
		: 'bg-transparent'}"
>
	<div class="flex items-center gap-6">
		<a href={resolve('/')}>
			<img
				src={logoText}
				alt="LisekMovie"
				class="transition-all duration-300 {scrolled ? 'h-6 md:h-8' : 'h-8 md:h-10'}"
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

	<UserMenu {username} />
</header>
