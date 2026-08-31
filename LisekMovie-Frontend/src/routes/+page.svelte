<!--
  Temporary Faza 1 scaffold-verification page, now also exercising the Faza
  3 sign-in state: proves the brand palette (Tailwind @theme tokens), the
  logo asset pipeline, and that a signed-in session round-trips end to end.
  Replaced entirely by the real Home screen in Faza 4 (see roadmap).
-->
<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { buildAuthHeader, clearSessionCookie, getSessionCookieClientSide } from '$lib/auth';
	import logoText from '$lib/assets/logo_text.svg';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	let loggingOut = $state(false);

	async function handleLogout() {
		loggingOut = true;
		const token = getSessionCookieClientSide();

		if (token) {
			try {
				await fetch('/api/auth/logout', { method: 'POST', headers: buildAuthHeader(token) });
			} catch {
				// The session is being cleared client-side regardless — a
				// failed logout call just means the (now-abandoned) server
				// session outlives the cookie until its own 30-day expiry,
				// not that the user stays signed in on this browser.
			}
		}

		clearSessionCookie();
		await goto(resolve('/'), { invalidateAll: true });
	}
</script>

<main class="flex min-h-screen flex-col items-center justify-center gap-6 bg-base px-6">
	<img src={logoText} alt="LisekMovie" class="w-64" />
	<p class="text-sm tracking-wide text-gold uppercase">Faza 1 — toolchain scaffold</p>

	{#if data.user}
		<div class="flex flex-col items-center gap-2">
			<p class="text-sm text-neutral-400">
				{t('auth.logged_in_as', { username: data.user.username })}
			</p>
			<button
				type="button"
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
				disabled={loggingOut}
				onclick={handleLogout}
			>
				{loggingOut ? t('auth.logging_out') : t('auth.log_out')}
			</button>
		</div>
	{/if}
</main>
