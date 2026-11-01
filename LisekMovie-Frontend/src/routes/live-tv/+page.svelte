<!--
  Faza 20 — Live TV: the programme guide (EPG). Channels down the left
  (sticky while the timeline scrolls), a time axis across the top, every
  programme as a block sized by its length, a line at the current time.
  A channel, or what is on it now, starts the live player; a later
  programme shows its details. Groups from the playlist filter the list.
-->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchGuide,
		guideStart,
		logoSrc,
		programBox,
		progressOf,
		PX_PER_MINUTE,
		type Guide,
		type GuideChannel,
		type LiveProgram
	} from '$lib/livetv/api';
	import Icon from '$lib/components/Icon.svelte';

	const HOURS = 24;
	const PAST_MINUTES = 90;

	let token = $state('');
	let guide = $state<Guide | null>(null);
	let loading = $state(true);
	let failed = $state(false);
	let group = $state<string | null>(null);
	let selected = $state<{ channel: GuideChannel; program: LiveProgram } | null>(null);
	let now = $state(Date.now());
	let scroller: HTMLDivElement | undefined = $state();
	const brokenLogos = $state<Record<string, boolean>>({});

	const start = $derived(guide ? new Date(guide.start) : new Date());
	const end = $derived(guide ? new Date(guide.end) : new Date());
	const width = $derived(((end.getTime() - start.getTime()) / 60000) * PX_PER_MINUTE);
	const nowLeft = $derived(((now - start.getTime()) / 60000) * PX_PER_MINUTE);
	const groups = $derived(
		[
			...new Set((guide?.channels ?? []).map((c) => c.group_title).filter((g): g is string => !!g))
		].sort((a, b) => a.localeCompare(b))
	);
	const channels = $derived(
		(guide?.channels ?? []).filter((c) => group === null || c.group_title === group)
	);
	/** Half-hour ticks on the axis. */
	const ticks = $derived.by(() => {
		const list: { left: number; label: string; hour: boolean }[] = [];
		if (!guide) return list;
		const format = new Intl.DateTimeFormat(getLocale(), { hour: '2-digit', minute: '2-digit' });
		for (let time = start.getTime(); time < end.getTime(); time += 30 * 60000) {
			const date = new Date(time);
			list.push({
				left: ((time - start.getTime()) / 60000) * PX_PER_MINUTE,
				label: format.format(date),
				hour: date.getMinutes() === 0
			});
		}
		return list;
	});

	function clock(iso: string) {
		return new Intl.DateTimeFormat(getLocale(), { hour: '2-digit', minute: '2-digit' }).format(
			new Date(iso)
		);
	}

	function day(iso: string) {
		return new Intl.DateTimeFormat(getLocale(), {
			weekday: 'long',
			day: 'numeric',
			month: 'long'
		}).format(new Date(iso));
	}

	function onNow(program: LiveProgram) {
		return Date.parse(program.starts_at) <= now && Date.parse(program.ends_at) > now;
	}

	async function scrollToNow(smooth = false) {
		await tick();
		scroller?.scrollTo({ left: Math.max(0, nowLeft - 120), behavior: smooth ? 'smooth' : 'auto' });
	}

	async function load() {
		loading = true;
		failed = false;
		try {
			guide = await fetchGuide(token, guideStart(new Date(), PAST_MINUTES), HOURS);
			now = Date.now();
			loading = false;
			await scrollToNow();
		} catch {
			failed = true;
			loading = false;
		}
	}

	function watch(channel: GuideChannel) {
		void goto(resolve('/live-tv/[channel]', { channel: channel.id }));
	}

	function pick(channel: GuideChannel, program: LiveProgram) {
		if (onNow(program)) watch(channel);
		else selected = { channel, program };
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		void load();
		const timer = setInterval(() => (now = Date.now()), 30_000);
		return () => clearInterval(timer);
	});
</script>

<svelte:head><title>{t('livetv.title')} · LisekMovie</title></svelte:head>

<main class="min-h-screen bg-base pt-20 pb-10 text-white">
	<header class="flex flex-wrap items-end justify-between gap-4 px-4 md:px-8">
		<div>
			<h1 class="text-2xl font-semibold md:text-3xl">{t('livetv.title')}</h1>
			<p class="mt-1 text-sm text-neutral-400 first-letter:uppercase">
				{day(new Date(now).toISOString())}
			</p>
		</div>
		<div class="flex items-center gap-2">
			<button
				type="button"
				onclick={() => scrollToNow(true)}
				class="rounded-full bg-gold px-4 py-1.5 text-sm font-semibold text-black hover:bg-gold-dim"
				>{t('livetv.now')}</button
			>
			<button
				type="button"
				onclick={load}
				aria-label={t('livetv.reload')}
				title={t('livetv.reload')}
				class="grid size-9 place-items-center rounded-full ring-1 ring-white/15 hover:bg-white/10"
			>
				<Icon name="refresh" class="size-4.5" />
			</button>
		</div>
	</header>

	{#if groups.length > 1}
		<div
			class="mt-4 flex gap-2 overflow-x-auto px-4 pb-1 md:px-8"
			role="radiogroup"
			aria-label={t('livetv.groups')}
		>
			<button
				type="button"
				role="radio"
				aria-checked={group === null}
				class="chip {group === null ? 'chip-on' : ''}"
				onclick={() => (group = null)}>{t('livetv.all_channels')}</button
			>
			{#each groups as name (name)}
				<button
					type="button"
					role="radio"
					aria-checked={group === name}
					class="chip {group === name ? 'chip-on' : ''}"
					onclick={() => (group = name)}>{name}</button
				>
			{/each}
		</div>
	{/if}

	{#if loading}
		<p class="px-4 pt-10 text-sm text-neutral-400 md:px-8" role="status">{t('common.loading')}</p>
	{:else if failed}
		<p class="px-4 pt-10 text-sm text-red-300 md:px-8" role="alert">{t('livetv.load_failed')}</p>
	{:else if !guide || guide.channels.length === 0}
		<div
			class="mx-4 mt-10 rounded-xl border border-dashed border-white/15 p-8 text-center text-neutral-400 md:mx-8"
		>
			<Icon name="tv" class="mx-auto mb-2 size-8" />
			<p>{t('livetv.no_channels')}</p>
		</div>
	{:else}
		<div
			bind:this={scroller}
			class="guide mt-4 overflow-auto border-y border-white/10"
			style:--axis-width={`${width}px`}
		>
			<div class="relative" style:width={`calc(var(--channel-col) + ${width}px)`}>
				<!-- Time axis -->
				<div class="sticky top-0 z-20 flex h-9 bg-neutral-950/95 backdrop-blur">
					<div
						class="sticky left-0 z-30 w-[var(--channel-col)] shrink-0 border-r border-white/10 bg-neutral-950"
					></div>
					<div class="relative h-full" style:width={`${width}px`}>
						{#each ticks as mark (mark.left)}
							<span
								class="absolute top-0 flex h-full items-center border-l pl-1.5 text-xs tabular-nums {mark.hour
									? 'border-white/25 text-neutral-200'
									: 'border-white/10 text-neutral-500'}"
								style:left={`${mark.left}px`}>{mark.label}</span
							>
						{/each}
					</div>
				</div>

				{#each channels as channel (channel.id)}
					<div class="flex h-[4.5rem] border-b border-white/5">
						<button
							type="button"
							onclick={() => watch(channel)}
							class="sticky left-0 z-10 flex w-[var(--channel-col)] shrink-0 items-center gap-2.5 border-r border-white/10 bg-neutral-950 px-2 text-left hover:bg-neutral-900 focus-visible:bg-neutral-900 md:px-3"
							aria-label={t('livetv.watch_channel', { name: channel.name })}
						>
							{#if channel.logo_url && !brokenLogos[channel.id]}
								<img
									src={logoSrc(channel, token)}
									alt=""
									loading="lazy"
									class="h-9 w-12 shrink-0 object-contain"
									onerror={() => (brokenLogos[channel.id] = true)}
								/>
							{:else}
								<span
									class="grid h-9 w-12 shrink-0 place-items-center rounded bg-white/10 text-xs font-semibold"
									>{channel.number ?? channel.name.slice(0, 3)}</span
								>
							{/if}
							<span class="hidden min-w-0 md:block">
								<span class="block truncate text-sm font-medium">{channel.name}</span>
								{#if channel.number}<span class="text-xs text-neutral-500">{channel.number}</span
									>{/if}
							</span>
						</button>
						<div class="relative h-full" style:width={`${width}px`}>
							{#each channel.programs as program (program.id)}
								{@const box = programBox(program, start, end)}
								{#if box}
									{@const live = onNow(program)}
									<button
										type="button"
										onclick={() => pick(channel, program)}
										class="program absolute top-1 bottom-1 overflow-clip rounded-md py-1.5 text-left transition-colors {live
											? 'bg-gold/20 ring-1 ring-gold/60 hover:bg-gold/30'
											: Date.parse(program.ends_at) <= now
												? 'bg-white/[0.03] text-neutral-500 hover:bg-white/10'
												: 'bg-white/[0.07] hover:bg-white/15'}"
										style:left={`${box.left + 1}px`}
										style:width={`${Math.max(box.width - 2, 2)}px`}
										title={`${clock(program.starts_at)}–${clock(program.ends_at)} ${program.title}`}
									>
										<!-- Sticks to the channel column, so a programme that began
										     before the visible part still shows its title. -->
										<span class="program-text sticky inline-block max-w-full px-2">
											<span class="block truncate text-sm font-medium">{program.title}</span>
											<span class="block truncate text-xs text-neutral-400 tabular-nums"
												>{clock(program.starts_at)}–{clock(program.ends_at)}</span
											>
										</span>
										{#if live}
											<span class="absolute inset-x-0 bottom-0 h-0.5 bg-white/10"
												><span
													class="block h-full bg-gold"
													style:width={`${progressOf(program, now) * 100}%`}
												></span></span
											>
										{/if}
									</button>
								{/if}
							{:else}
								<span
									class="absolute inset-y-1 left-1 flex items-center px-2 text-xs text-neutral-600"
									>{t('livetv.no_guide')}</span
								>
							{/each}
						</div>
					</div>
				{/each}

				{#if nowLeft > 0 && nowLeft < width}
					<div
						class="pointer-events-none absolute top-0 bottom-0 z-[15] w-0.5 bg-red-500 shadow-[0_0_8px_rgb(239_68_68/0.8)]"
						style:left={`calc(var(--channel-col) + ${nowLeft}px)`}
						aria-hidden="true"
					></div>
				{/if}
			</div>
		</div>
	{/if}
</main>

{#if selected}
	<div
		role="presentation"
		class="fixed inset-0 z-50 flex items-end justify-center bg-black/60 p-4 sm:items-center"
		onclick={(event) => event.target === event.currentTarget && (selected = null)}
	>
		<div
			role="dialog"
			aria-modal="true"
			aria-labelledby="program-title"
			class="w-full max-w-lg rounded-2xl border border-white/10 bg-surface p-6"
		>
			<p class="text-xs font-semibold tracking-wider text-gold uppercase">
				{selected.channel.name}
			</p>
			<h2 id="program-title" class="mt-1 text-xl font-semibold">{selected.program.title}</h2>
			{#if selected.program.subtitle}<p class="text-neutral-300">
					{selected.program.subtitle}
				</p>{/if}
			<p class="mt-1 text-sm text-neutral-400 tabular-nums first-letter:uppercase">
				{day(selected.program.starts_at)} · {clock(selected.program.starts_at)}–{clock(
					selected.program.ends_at
				)}
				{#if selected.program.category}· {selected.program.category}{/if}
			</p>
			{#if selected.program.description}<p class="mt-4 text-sm leading-relaxed text-neutral-300">
					{selected.program.description}
				</p>{/if}
			<div class="mt-6 flex justify-end gap-2">
				<button
					type="button"
					class="rounded-md border border-neutral-700 px-3 py-2 text-sm hover:border-gold-dim"
					onclick={() => (selected = null)}>{t('admin.editor_close')}</button
				>
				<button
					type="button"
					class="flex items-center gap-2 rounded-md bg-gold px-4 py-2 text-sm font-semibold text-black hover:bg-gold-dim"
					onclick={() => selected && watch(selected.channel)}
				>
					<Icon name="play" class="size-4" />
					{t('livetv.watch_now')}
				</button>
			</div>
		</div>
	</div>
{/if}

<style>
	.guide {
		--channel-col: 4.5rem;
		max-height: calc(100vh - 13rem);
		overscroll-behavior: contain;
	}
	@media (min-width: 768px) {
		.guide {
			--channel-col: 13rem;
		}
	}
	.program-text {
		left: var(--channel-col);
	}
	.chip {
		flex-shrink: 0;
		border-radius: 9999px;
		padding: 0.3rem 0.85rem;
		font-size: 0.85rem;
		box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.15);
		color: rgb(212 212 212);
	}
	.chip:hover {
		background: rgb(255 255 255 / 0.08);
	}
	.chip-on {
		background: white;
		color: black;
		box-shadow: none;
	}
	.chip-on:hover {
		background: white;
	}
</style>
