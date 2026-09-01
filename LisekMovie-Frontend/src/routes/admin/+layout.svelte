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

	const tabs = [
		{ href: '/admin/plugins', labelKey: 'admin.tab_plugins' },
		{ href: '/admin/libraries', labelKey: 'admin.tab_libraries' }
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
		<div class="mx-auto flex max-w-2xl gap-1">
			{#each tabs as tab (tab.href)}
				{@const active = page.url.pathname.startsWith(tab.href)}
				<a
					href={resolve(tab.href)}
					class="border-b-2 px-3 py-3 text-sm font-medium transition-colors {active
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
