<!--
  Faza 19: a round account picture. Shows the uploaded avatar, or the
  username's initial on gold when there is none (or it fails to load).
  `online` adds the status dot: green online, grey offline; leave it
  undefined to show no dot.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { avatarSrc } from '$lib/users/profile';

	interface Props {
		username: string;
		url?: string | null;
		/** Tailwind size and text classes, e.g. `size-10 text-base`. */
		class?: string;
		online?: boolean;
	}

	let { username, url = null, class: className = 'size-8 text-sm', online }: Props = $props();

	let failed = $state<string | null>(null);
	// The session is only readable in the browser: the server-rendered page
	// (and hydration) shows the initial, then the picture is swapped in.
	let mounted = $state(false);
	onMount(() => (mounted = true));
	const src = $derived(mounted && url ? avatarSrc(url, getSessionCookieClientSide()) : null);
	const initial = $derived((username.trim().charAt(0) || '?').toUpperCase());
</script>

<span class="relative inline-flex shrink-0 {className}">
	{#if src && failed !== src}
		<img
			{src}
			alt=""
			loading="lazy"
			decoding="async"
			onerror={() => (failed = src)}
			class="size-full rounded-full object-cover ring-1 ring-white/15"
		/>
	{:else}
		<span
			aria-hidden="true"
			class="flex size-full items-center justify-center rounded-full bg-gradient-to-br from-gold to-gold-dim font-semibold text-black ring-1 ring-white/15 select-none"
		>
			{initial}
		</span>
	{/if}
	{#if online !== undefined}
		<span
			role="img"
			aria-label={online ? t('profile.online') : t('profile.offline')}
			title={online ? t('profile.online') : t('profile.offline')}
			class="absolute right-0 bottom-0 size-[28%] min-h-2.5 min-w-2.5 rounded-full ring-2 ring-surface {online
				? 'bg-emerald-500 shadow-[0_0_6px_rgb(16_185_129/0.8)]'
				: 'bg-neutral-500'}"
		></span>
	{/if}
</span>
