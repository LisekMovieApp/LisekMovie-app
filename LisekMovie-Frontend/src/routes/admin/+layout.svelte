<!--
  Shared chrome for everything under /admin/**: a small tab bar so an admin
  can move between "Wtyczki" (providers) and "Biblioteki" (libraries)
  without going back through the account menu each time. The actual
  admin-only guard lives in this route's `+layout.ts` (data this component
  never needs directly — SvelteKit runs it regardless of whether the
  `.svelte` sibling reads its `data`), so this file is presentation only.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';

	let { children } = $props();

	// Faza 14: the tab strip scrolls without a visible scrollbar (swipe on
	// touch screens); with a mouse, faded edges show there is more to the
	// side. The fades follow the scroll position.
	let scroller = $state<HTMLElement>();
	let atStart = $state(true);
	let atEnd = $state(true);

	function measure() {
		if (!scroller) return;
		atStart = scroller.scrollLeft <= 1;
		atEnd = scroller.scrollLeft + scroller.clientWidth >= scroller.scrollWidth - 1;
	}

	$effect(() => {
		if (!scroller) return;
		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(scroller);
		return () => observer.disconnect();
	});

	// Keep the open tab in view when the page changes (narrow screens).
	$effect(() => {
		void page.url.pathname;
		const active = scroller?.querySelector<HTMLElement>('[aria-current="page"]');
		active?.scrollIntoView?.({ block: 'nearest', inline: 'center' });
	});

	const mask = $derived(
		atStart && atEnd
			? 'none'
			: `linear-gradient(to right, ${atStart ? '#000' : 'transparent'} 0, #000 2.5rem, #000 calc(100% - 2.5rem), ${atEnd ? '#000' : 'transparent'} 100%)`
	);

	const tabs = [
        { href: '/admin/users', labelKey: 'admin.tab_users' },
		{ href: '/admin/plugins', labelKey: 'admin.tab_plugins' },
		{ href: '/admin/stats', labelKey: 'player.stats' },
        { href: '/admin/playback', labelKey: 'admin.tab_hardware' },
		{ href: '/admin/trickplay', labelKey: 'trickplay_admin.tab' },
        { href: '/admin/notifications', labelKey: 'notifications.title' },
		{ href: '/admin/games', labelKey: 'nav.games' },
		{ href: '/admin/libraries', labelKey: 'admin.tab_libraries' },
		{ href: '/admin/media/fallback', labelKey: 'admin.tab_fallback' },
		{ href: '/admin/logs', labelKey: 'logs.tab' }
	] as const;
</script>

<!--
  `pt-24` clears the fixed Navbar (up to ~88px tall unscrolled: h-16 logo +
  py-3) — without it this tab bar sits directly underneath the navbar's own
  interactive area and swallows clicks meant for these links, since it's
  the very first thing on the page with nothing above it to push it down
  (every other page's first interactive content is further down, past its
  own heading, and never hit this).
-->
<div class="pt-24">
	<nav class="border-b border-white/10 bg-surface/40 px-4 md:px-8">
		<div
			bind:this={scroller}
			onscroll={measure}
			class="admin-tabs mx-auto flex max-w-5xl gap-1 overflow-x-auto"
			style:--tabs-mask={mask}
		>
			{#each tabs as tab (tab.href)}
				{@const active = page.url.pathname.startsWith(tab.href)}
				<a
					href={resolve(tab.href)}
					aria-current={active ? 'page' : undefined}
					class="border-b-2 px-3 py-3 text-sm font-medium whitespace-nowrap transition-colors {active
						? 'border-gold text-gold'
						: 'border-transparent text-neutral-400 hover:text-neutral-200'}"
				>
					{t(tab.labelKey)}
				</a>
			{/each}
		</div>
	</nav>

	{@render children()}
</div>

<style>
	/* No scrollbar anywhere; swipe and wheel still scroll. */
	.admin-tabs {
		scrollbar-width: none;
		-ms-overflow-style: none;
		-webkit-overflow-scrolling: touch;
		overscroll-behavior-x: contain;
		scroll-behavior: smooth;
	}
	.admin-tabs::-webkit-scrollbar {
		display: none;
	}
	/* Mouse and trackpad (no touch): fade the edge with more tabs behind it. */
	@media (hover: hover) and (pointer: fine) {
		.admin-tabs {
			-webkit-mask-image: var(--tabs-mask);
			mask-image: var(--tabs-mask);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.admin-tabs {
			scroll-behavior: auto;
		}
	}
</style>
