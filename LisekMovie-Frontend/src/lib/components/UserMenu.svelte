<!--
  Avatar button in the navbar's top-right corner. Click toggles a small
  dropdown with the signed-in username and a sign-out action — the same
  logout flow the Faza 3 home-page scaffold used, just relocated here now
  that the navbar is the permanent home for account controls.
-->
<script lang="ts">
	import BrowserNotifications from './BrowserNotifications.svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { buildAuthHeader, clearSessionCookie, getSessionCookieClientSide } from '$lib/auth';

	interface Props {
		username: string;
		isAdmin: boolean;
	}

	let { username, isAdmin }: Props = $props();

	let open = $state(false);
	let loggingOut = $state(false);
	let menuEl: HTMLDivElement | undefined = $state();

	function toggleOpen() {
		open = !open;
	}

	function handleWindowClick(event: MouseEvent) {
		if (open && menuEl && !menuEl.contains(event.target as Node)) {
			open = false;
		}
	}

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

	const initial = $derived(username.charAt(0).toUpperCase());
</script>

<svelte:window onclick={handleWindowClick} />

<div class="relative" bind:this={menuEl}>
	<button
		type="button"
		aria-label={t('nav.account_menu')}
		aria-expanded={open}
		onclick={toggleOpen}
		class="flex size-8 items-center justify-center rounded-md bg-gold text-sm font-semibold text-black transition-colors hover:bg-gold-dim"
	>
		{initial}
	</button>

		<div
			hidden={!open}
			class="absolute right-0 mt-2 w-64 rounded-md border border-white/10 bg-surface p-3 shadow-lg"
		>
			<p class="truncate px-1 pb-2 text-sm text-neutral-300">
				{t('auth.logged_in_as', { username })}
			</p>
			{#key page.data.user?.id}<BrowserNotifications userId={page.data.user?.id ?? username} />{/key}
            <a href={resolve('/change-password')} class="mb-2 block rounded border border-neutral-700 px-3 py-2 text-sm">{t('auth.change_password')}</a>
            {#if isAdmin}
				<a
					href={resolve('/admin/plugins')}
					class="mb-1.5 block w-full rounded-md border border-neutral-700 px-3 py-1.5 text-left text-sm text-neutral-200 transition-colors hover:border-gold-dim"
				>
					{t('nav.admin_panel')}
				</a>
			{/if}
			<button
				type="button"
				disabled={loggingOut}
				onclick={handleLogout}
				class="w-full rounded-md border border-neutral-700 px-3 py-1.5 text-left text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
			>
				{loggingOut ? t('auth.logging_out') : t('auth.log_out')}
			</button>
		</div>
</div>
