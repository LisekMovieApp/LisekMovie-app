<!--
  Faza 20 — a watch-together link (`/together/ABC234`): finds the room and
  opens its player. A wrong or expired code shows a form to type another.
-->
<script lang="ts">
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchRoom, normalizeCode, RoomError } from '$lib/syncplay/room';
	import Icon from '$lib/components/Icon.svelte';

	let errorKey = $state<string | null>(null);
	let typed = $state('');
	let busy = $state(true);

	async function join(raw: string) {
		const code = normalizeCode(raw);
		if (!code) {
			errorKey = 'syncplay.errors.invalid_code';
			busy = false;
			return;
		}
		busy = true;
		errorKey = null;
		try {
			const room = await fetchRoom(getSessionCookieClientSide() ?? '', code);
			// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query
			await goto(resolve('/watch/[id]', { id: room.item_id }) + `?room=${room.code}`, {
				replaceState: true
			});
		} catch (error) {
			errorKey =
				error instanceof RoomError && error.code === 'room_not_found'
					? 'syncplay.errors.room_not_found'
					: 'syncplay.errors.join_failed';
			busy = false;
		}
	}

	$effect(() => {
		const code = page.params.code ?? '';
		typed = code;
		void join(code);
	});
</script>

<svelte:head><title>{t('syncplay.title')} · LisekMovie</title></svelte:head>

<main class="flex min-h-screen items-center justify-center bg-base px-4 pt-20 text-white">
	<section class="w-full max-w-sm rounded-2xl border border-white/10 bg-surface p-6 text-center">
		<span class="mx-auto mb-3 grid size-12 place-items-center rounded-full bg-gold/15 text-gold">
			<Icon name="users" class="size-6" />
		</span>
		<h1 class="text-xl font-semibold">{t('syncplay.title')}</h1>
		{#if busy && !errorKey}
			<p class="mt-2 text-sm text-neutral-400" role="status">{t('syncplay.joining')}</p>
		{:else}
			{#if errorKey}<p role="alert" class="mt-2 text-sm text-red-300">{t(errorKey)}</p>{/if}
			<form
				class="mt-4 flex gap-2"
				onsubmit={(event) => {
					event.preventDefault();
					void join(typed);
				}}
			>
				<input
					bind:value={typed}
					maxlength="9"
					autocomplete="off"
					aria-label={t('syncplay.code_label')}
					placeholder={t('syncplay.code_label')}
					class="min-w-0 flex-1 rounded-md border border-neutral-700 bg-black px-3 py-2 text-center font-mono text-lg tracking-widest uppercase"
				/>
				<button class="rounded-md bg-gold px-4 py-2 font-semibold text-black"
					>{t('syncplay.join')}</button
				>
			</form>
		{/if}
	</section>
</main>
