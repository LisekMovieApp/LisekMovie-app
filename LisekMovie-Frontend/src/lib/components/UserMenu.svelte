<!--
  Account menu in the navbar's top-right corner (Faza 19). The round avatar
  opens a dropdown: who is signed in and with which role, Profile settings,
  the browser-notification switch for this device, the Admin Panel (only
  for administrators), installing the app, and Log out at the very bottom.
  The panel is hidden rather than removed while closed, so the
  notification socket inside it keeps running.
-->
<script lang="ts">
	import BrowserNotifications from './BrowserNotifications.svelte';
	import Avatar from './Avatar.svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { buildAuthHeader, clearSessionCookie, getSessionCookieClientSide } from '$lib/auth';
	import { install, promptInstall } from '$lib/pwa/install.svelte';
	import Icon from './Icon.svelte';

	interface Props {
		username: string;
		isAdmin: boolean;
		avatarUrl?: string | null;
	}

	let { username, isAdmin, avatarUrl = null }: Props = $props();

	let open = $state(false);
	let loggingOut = $state(false);
	let iosHelp = $state(false);
	let menuEl: HTMLDivElement | undefined = $state();
	let triggerEl: HTMLButtonElement | undefined = $state();

	const ITEM =
		'flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm text-neutral-200 transition-colors hover:bg-white/10 hover:text-white focus-visible:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50';

	function toggleOpen() {
		open = !open;
	}

	function close(focusTrigger = false) {
		open = false;
		if (focusTrigger) triggerEl?.focus();
	}

	function handleWindowClick(event: MouseEvent) {
		if (open && menuEl && !menuEl.contains(event.target as Node)) close();
	}

	function handleKey(event: KeyboardEvent) {
		if (open && event.key === 'Escape') close(true);
	}

	$effect(() => {
		// Any navigation (a menu link) closes the menu.
		void page.url.pathname;
		open = false;
	});

	async function handleLogout() {
		loggingOut = true;
		const token = getSessionCookieClientSide();

		if (token) {
			try {
				await fetch('/api/auth/logout', { method: 'POST', headers: buildAuthHeader(token) });
			} catch {
				// The session is cleared client-side regardless — a failed logout
				// call just means the (now-abandoned) server session outlives the
				// cookie until its own 30-day expiry, not that the user stays
				// signed in on this browser.
			}
		}

		clearSessionCookie();
		await goto(resolve('/login'), { invalidateAll: true });
	}
</script>

<svelte:window onclick={handleWindowClick} onkeydown={handleKey} />

<div class="relative" bind:this={menuEl}>
	<button
		type="button"
		bind:this={triggerEl}
		aria-label={t('nav.account_menu')}
		aria-haspopup="true"
		aria-expanded={open}
		onclick={toggleOpen}
		class="flex rounded-full ring-2 transition-all hover:ring-gold/70 focus-visible:ring-gold {open
			? 'ring-gold'
			: 'ring-transparent'}"
	>
		<Avatar {username} url={avatarUrl} class="size-9 text-sm" />
	</button>

	<div
		hidden={!open}
		class="menu-panel absolute right-0 z-50 mt-3 w-72 origin-top-right overflow-hidden rounded-2xl border border-white/10 bg-surface/95 shadow-2xl shadow-black/60 backdrop-blur-xl"
	>
		<div class="flex items-center gap-3 border-b border-white/10 bg-gradient-to-br from-gold/10 to-transparent px-4 py-4">
			<Avatar {username} url={avatarUrl} class="size-12 text-lg" />
			<div class="min-w-0">
				<p class="truncate text-sm text-neutral-300">
					{t('profile.menu.signed_in_as')} <strong class="font-semibold text-white">{username}</strong>
				</p>
				<p class="mt-0.5 flex items-center gap-1 text-xs font-light text-neutral-400">
					{#if isAdmin}<Icon name="shield" class="size-3.5 text-gold" />{/if}
					{t('profile.menu.role', {
						role: isAdmin ? t('accounts.administrator') : t('accounts.user')
					})}
				</p>
			</div>
		</div>

		<div class="flex flex-col gap-0.5 p-2">
			<a href={resolve('/settings/profile')} class={ITEM}>
				<Icon name="settings" class="size-4.5 shrink-0 text-neutral-400" />
				{t('profile.menu.settings')}
			</a>
			{#key page.data.user?.id}<BrowserNotifications userId={page.data.user?.id ?? username} />{/key}
			{#if install.available}
				<button type="button" onclick={() => promptInstall()} class={ITEM}>
					<Icon name="download" class="size-4.5 shrink-0 text-gold" />
					{t('pwa.install')}
				</button>
			{:else if install.ios}
				<button type="button" onclick={() => (iosHelp = !iosHelp)} aria-expanded={iosHelp} class={ITEM}>
					<Icon name="download" class="size-4.5 shrink-0 text-gold" />
					{t('pwa.install_ios')}
				</button>
				{#if iosHelp}<p class="px-3 pb-1 pl-10.5 text-xs text-neutral-400">{t('pwa.install_ios_hint')}</p>{/if}
			{/if}
			{#if isAdmin}
				<a href={resolve('/admin/plugins')} class={ITEM}>
					<Icon name="shield" class="size-4.5 shrink-0 text-neutral-400" />
					{t('nav.admin_panel')}
				</a>
			{/if}
		</div>

		<div class="border-t border-white/10 p-2">
			<button
				type="button"
				disabled={loggingOut}
				onclick={handleLogout}
				class="{ITEM} text-red-300 hover:bg-red-500/10 hover:text-red-200 focus-visible:bg-red-500/10"
			>
				<Icon name="logout" class="size-4.5 shrink-0" />
				{loggingOut ? t('auth.logging_out') : t('auth.log_out')}
			</button>
		</div>
	</div>
</div>

<style>
	.menu-panel:not([hidden]) {
		animation: menu-in 140ms ease-out;
	}
	@keyframes menu-in {
		from {
			opacity: 0;
			transform: translateY(-4px) scale(0.98);
		}
	}
	@media (prefers-reduced-motion: reduce) {
		.menu-panel:not([hidden]) {
			animation: none;
		}
	}
</style>
