<!--
  Faza 20 — "Watch with others": opens a room for one film or episode and
  shows its code and link. Everyone who opens the link (signed in, allowed
  to see the title) lands in the same player; play, pause and seek are
  shared.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { createRoom, roomLink, RoomError, type CreatedRoom } from '$lib/syncplay/room';
	import Icon from './Icon.svelte';

	interface Props {
		itemId: string;
		/** What is being watched, for the heading. */
		title: string;
		onclose: () => void;
	}

	let { itemId, title, onclose }: Props = $props();

	let room = $state<CreatedRoom | null>(null);
	let errorKey = $state<string | null>(null);
	let copied = $state(false);
	const link = $derived(room ? roomLink(room.code) : '');

	onMount(() => {
		createRoom(getSessionCookieClientSide() ?? '', itemId)
			.then((created) => (room = created))
			.catch((error) => {
				const code = error instanceof RoomError ? error.code : '';
				errorKey =
					code === 'too_many_rooms'
						? 'syncplay.errors.too_many_rooms'
						: 'syncplay.errors.create_failed';
			});
	});

	async function copy() {
		try {
			await navigator.clipboard.writeText(link);
			copied = true;
			setTimeout(() => (copied = false), 2000);
		} catch {
			copied = false;
		}
	}

	async function start() {
		if (!room) return;
		// eslint-disable-next-line svelte/no-navigation-without-resolve -- resolved path plus a query
		await goto(resolve('/watch/[id]', { id: room.item_id }) + `?room=${room.code}`);
	}

	function onKey(event: KeyboardEvent) {
		if (event.key === 'Escape') onclose();
	}
</script>

<svelte:window onkeydown={onKey} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
	onclick={(event) => event.target === event.currentTarget && onclose()}
>
	<div
		role="dialog"
		aria-modal="true"
		aria-labelledby="together-title"
		class="w-full max-w-md rounded-2xl border border-white/10 bg-surface p-6 text-neutral-200 shadow-2xl"
	>
		<div class="mb-4 flex items-start gap-3">
			<span class="grid size-11 shrink-0 place-items-center rounded-full bg-gold/15 text-gold">
				<Icon name="users" class="size-6" />
			</span>
			<div class="min-w-0">
				<h2 id="together-title" class="text-lg font-semibold text-white">{t('syncplay.title')}</h2>
				<p class="truncate text-sm text-neutral-400">{title}</p>
			</div>
		</div>

		{#if errorKey}
			<p role="alert" class="text-sm text-red-300">{t(errorKey)}</p>
		{:else if !room}
			<p class="text-sm text-neutral-400">{t('syncplay.creating')}</p>
		{:else}
			<p class="text-sm text-neutral-300">{t('syncplay.share_hint')}</p>
			<p
				class="my-4 text-center font-mono text-4xl font-bold tracking-[0.3em] text-white"
				aria-label={t('syncplay.room_code', { code: room.code })}
			>
				{room.code}
			</p>
			<div class="flex gap-2">
				<input
					readonly
					value={link}
					aria-label={t('syncplay.link')}
					class="min-w-0 flex-1 rounded-md border border-neutral-700 bg-black px-3 py-2 text-sm"
					onfocus={(event) => (event.currentTarget as HTMLInputElement).select()}
				/>
				<button
					type="button"
					onclick={copy}
					class="shrink-0 rounded-md border border-neutral-700 px-3 py-2 text-sm hover:border-gold-dim"
				>
					{copied ? t('syncplay.link_copied') : t('syncplay.copy_link')}
				</button>
			</div>
			<p class="mt-3 text-xs text-neutral-500">{t('syncplay.how_it_works')}</p>
		{/if}

		<div class="mt-6 flex justify-end gap-2">
			<button
				type="button"
				onclick={onclose}
				class="rounded-md border border-neutral-700 px-3 py-2 text-sm hover:border-gold-dim"
				>{t('admin.cancel')}</button
			>
			<button
				type="button"
				disabled={!room}
				onclick={start}
				class="flex items-center gap-2 rounded-md bg-gold px-4 py-2 text-sm font-semibold text-black hover:bg-gold-dim disabled:opacity-50"
			>
				<Icon name="play" class="size-4" />
				{t('syncplay.start')}
			</button>
		</div>
	</div>
</div>
