<!--
  Faza 26 — the programme guide as the administrator checks it: every
  channel (own and IPTV) against the next hours. Viewers see the same grid
  on the Live TV page.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchGuide, guideStart, programBox, PX_PER_MINUTE, type Guide } from '$lib/livetv/api';

	const HOURS = 8;
	let guide = $state<Guide | null>(null);
	let loading = $state(true);
	let failed = $state(false);
	let now = $state(Date.now());

	const start = $derived(guide ? new Date(guide.start) : new Date());
	const end = $derived(guide ? new Date(guide.end) : new Date());
	const width = $derived(((end.getTime() - start.getTime()) / 60000) * PX_PER_MINUTE);
	const nowLeft = $derived(((now - start.getTime()) / 60000) * PX_PER_MINUTE);
	const ticks = $derived.by(() => {
		const list: { left: number; label: string }[] = [];
		if (!guide) return list;
		const format = new Intl.DateTimeFormat(getLocale(), { hour: '2-digit', minute: '2-digit' });
		for (let time = start.getTime(); time < end.getTime(); time += 60 * 60000) {
			list.push({ left: ((time - start.getTime()) / 60000) * PX_PER_MINUTE, label: format.format(new Date(time)) });
		}
		return list;
	});

	onMount(() => {
		const token = getSessionCookieClientSide() ?? '';
		fetchGuide(token, guideStart(new Date(), 30), HOURS)
			.then((g) => (guide = g))
			.catch(() => (failed = true))
			.finally(() => (loading = false));
		const timer = setInterval(() => (now = Date.now()), 30_000);
		return () => clearInterval(timer);
	});
</script>

<svelte:head><title>{t('tv_admin.tab_guide')} · LisekMovie</title></svelte:head>

<main class="min-h-screen bg-base px-4 py-8 md:px-8">
	<div class="mx-auto max-w-5xl">
		<div class="flex flex-wrap items-end justify-between gap-3">
			<div>
				<h1 class="text-2xl font-semibold text-neutral-100">{t('tv_admin.tab_guide')}</h1>
				<p class="mt-1 text-sm text-neutral-400">{t('tv_admin.guide_hint', { hours: HOURS })}</p>
			</div>
			<a href={resolve('/live-tv')} class="text-sm text-gold hover:underline">{t('tv_admin.open_viewer')}</a>
		</div>

		{#if loading}
			<p class="mt-6 text-sm text-neutral-400">{t('admin.loading')}</p>
		{:else if failed || !guide}
			<p class="mt-6 text-sm text-red-400">{t('admin.load_failed')}</p>
		{:else if guide.channels.length === 0}
			<p class="mt-6 text-sm text-neutral-400">{t('tv_admin.guide_empty')}</p>
		{:else}
			<div class="mt-6 overflow-x-auto rounded-lg ring-1 ring-white/10">
				<div class="relative" style:width={`${width + 160}px`}>
					<div class="flex border-b border-white/10 bg-white/5 text-xs text-neutral-400">
						<div class="w-40 shrink-0 p-2"></div>
						<div class="relative h-7" style:width={`${width}px`}>
							{#each ticks as tick (tick.left)}
								<span class="absolute top-1.5" style:left={`${tick.left + 4}px`}>{tick.label}</span>
							{/each}
						</div>
					</div>
					{#each guide.channels as channel (channel.id)}
						<div class="flex border-b border-white/5">
							<div class="w-40 shrink-0 truncate p-2 text-sm text-neutral-200">
								<span class="text-gold">{channel.number ?? ''}</span>
								{channel.name}
							</div>
							<div class="relative h-12" style:width={`${width}px`}>
								{#each channel.programs as program (program.starts_at + program.title)}
									{@const box = programBox(program, start, end)}
									{#if box}
										<div
											class="absolute top-1 h-10 overflow-hidden rounded bg-white/10 px-2 py-1 text-xs text-neutral-200 ring-1 ring-black/40"
											style:left={`${box.left}px`}
											style:width={`${Math.max(box.width - 2, 2)}px`}
											title={program.title}
										>
											{program.title}
										</div>
									{/if}
								{/each}
								<span class="absolute inset-y-0 w-px bg-red-500/70" style:left={`${nowLeft}px`}></span>
							</div>
						</div>
					{/each}
				</div>
			</div>
		{/if}
	</div>
</main>
