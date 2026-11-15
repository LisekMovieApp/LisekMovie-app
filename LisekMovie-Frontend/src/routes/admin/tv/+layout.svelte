<!--
  Faza 26 — the "Telewizja" admin area: own channels, ads and bumpers,
  IPTV channels and the programme guide. A second tab strip under the
  admin one.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';

	let { children } = $props();

	const tabs = [
		{ href: '/admin/tv', labelKey: 'tv_admin.tab_channels' },
		{ href: '/admin/tv/fillers', labelKey: 'tv_admin.tab_fillers' },
		{ href: '/admin/tv/iptv', labelKey: 'tv_admin.tab_iptv' },
		{ href: '/admin/tv/guide', labelKey: 'tv_admin.tab_guide' }
	] as const;

	function isActive(href: string): boolean {
		const path = page.url.pathname;
		if (href === '/admin/tv') {
			return path === href || path.startsWith('/admin/tv/channel');
		}
		return path.startsWith(href);
	}
</script>

<nav class="mx-auto mt-6 flex max-w-5xl gap-2 overflow-x-auto px-4 md:px-8" aria-label={t('tv_admin.nav')}>
	{#each tabs as tab (tab.href)}
		<a
			href={resolve(tab.href)}
			aria-current={isActive(tab.href) ? 'page' : undefined}
			class="rounded-full px-4 py-1.5 text-sm whitespace-nowrap transition-colors {isActive(tab.href)
				? 'bg-gold text-black'
				: 'bg-white/5 text-neutral-300 hover:bg-white/10'}"
		>
			{t(tab.labelKey)}
		</a>
	{/each}
</nav>

{@render children()}
