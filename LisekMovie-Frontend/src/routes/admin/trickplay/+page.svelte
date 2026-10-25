<!--
  Admin → Miniatury (Faza 6): live progress of the trickplay worker
  (`GET /api/admin/trickplay`, polled every 3 s while the tab is visible).

  - Whole catalogue: one bar (ready + failed out of every video), counts
    and a rough remaining time from the worker's recent pace.
  - What is being generated right now, with a running clock; or why
    nothing is (turned off, FFmpeg missing, paused while someone is
    transcoding, all done).
  - One bar per library.
  - Recent failures with "Ponów" (`POST /api/admin/media/{id}/trickplay`).
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import Icon from '$lib/components/Icon.svelte';
	import { formatTime } from '$lib/player/format';
	import {
		POLL_MS,
		eta,
		fetchTrickplayStatus,
		percentDone,
		regenerateTrickplay,
		workerState,
		type TrickplayStatus
	} from '$lib/admin/trickplay';

	let status = $state<TrickplayStatus | null>(null);
	let failed = $state(false);
	let now = $state(Date.now());
	let retrying = $state<Record<string, boolean>>({});
	let token = '';

	async function refresh() {
		try {
			status = await fetchTrickplayStatus(token);
			failed = false;
		} catch {
			failed = true;
		}
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		void refresh();
		const poll = setInterval(() => {
			if (document.visibilityState === 'visible') void refresh();
		}, POLL_MS);
		const clock = setInterval(() => (now = Date.now()), 1000);
		const onVisible = () => document.visibilityState === 'visible' && void refresh();
		document.addEventListener('visibilitychange', onVisible);
		return () => {
			clearInterval(poll);
			clearInterval(clock);
			document.removeEventListener('visibilitychange', onVisible);
		};
	});

	async function retry(itemId: string) {
		retrying = { ...retrying, [itemId]: true };
		try {
			await regenerateTrickplay(token, itemId);
			await refresh();
		} catch {
			failed = true;
		} finally {
			retrying = { ...retrying, [itemId]: false };
		}
	}

	const total = $derived(status ? status.ready + status.failed + status.pending : 0);
	const percent = $derived(status ? percentDone(status.ready, status.failed, total) : 0);
	const phase = $derived(status ? workerState(status) : null);
	const remaining = $derived(
		status?.eta_seconds !== null && status?.eta_seconds !== undefined && status.pending > 0
			? eta(status.eta_seconds)
			: null
	);
	const elapsed = $derived(
		status?.current ? Math.max(0, (now - Date.parse(status.current.started_at)) / 1000) : 0
	);

	function itemLabel(title: string, season?: number | null, episode?: number | null): string {
		return season !== null && season !== undefined && episode !== null && episode !== undefined
			? `${title} · ${t('home.episode_short', { season, episode })}`
			: title;
	}

	const STATE_STYLE: Record<string, string> = {
		working: 'bg-gold/15 text-gold ring-gold/40',
		paused: 'bg-sky-500/15 text-sky-300 ring-sky-400/40',
		idle: 'bg-white/10 text-neutral-200 ring-white/20',
		done: 'bg-emerald-500/15 text-emerald-300 ring-emerald-400/40',
		disabled: 'bg-white/5 text-neutral-400 ring-white/10',
		no_ffmpeg: 'bg-red-500/15 text-red-300 ring-red-400/40'
	};
</script>

<svelte:head><title>{t('trickplay_admin.title')} · LisekMovie</title></svelte:head>

<main class="mx-auto max-w-5xl px-4 py-8 text-white md:px-8">
	<div class="mb-2 flex flex-wrap items-center justify-between gap-3">
		<h1 class="text-2xl font-semibold">{t('trickplay_admin.title')}</h1>
		{#if phase}
			<span
				class="flex items-center gap-2 rounded-full px-3 py-1 text-sm font-medium ring-1 {STATE_STYLE[
					phase
				]}"
				role="status"
			>
				{#if phase === 'working'}
					<span class="relative flex size-2.5">
						<span class="absolute inline-flex size-full animate-ping rounded-full bg-gold opacity-75"
						></span>
						<span class="relative inline-flex size-2.5 rounded-full bg-gold"></span>
					</span>
				{/if}
				{t(`trickplay_admin.state_${phase}`)}
			</span>
		{/if}
	</div>
	<p class="mb-6 text-sm text-neutral-400">{t('trickplay_admin.hint')}</p>

	{#if failed && !status}
		<p role="alert" class="rounded bg-red-950/40 p-4 text-red-200">{t('trickplay_admin.load_failed')}</p>
	{:else if !status}
		<div class="h-40 animate-pulse rounded-xl bg-surface"></div>
	{:else}
		<section class="rounded-xl border border-white/10 bg-surface/60 p-5 md:p-6">
			<div class="flex flex-wrap items-end justify-between gap-4">
				<div>
					<p class="text-5xl font-bold tabular-nums">{percent}%</p>
					<p class="mt-1 text-sm text-neutral-400">
						{t('trickplay_admin.done_of', { done: status.ready + status.failed, total })}
					</p>
				</div>
				{#if remaining}
					<p class="text-right text-sm text-neutral-300">
						<span class="block text-xs tracking-wide text-neutral-500 uppercase"
							>{t('trickplay_admin.eta')}</span
						>
						{#if remaining.kind === 'soon'}{t('trickplay_admin.eta_soon')}
						{:else if remaining.kind === 'minutes'}{t('trickplay_admin.eta_minutes', {
								minutes: remaining.minutes
							})}
						{:else}{t('trickplay_admin.eta_hours', {
								hours: remaining.hours,
								minutes: remaining.minutes
							})}{/if}
					</p>
				{/if}
			</div>

			<div
				class="mt-4 flex h-3 overflow-hidden rounded-full bg-white/10"
				role="progressbar"
				aria-valuemin="0"
				aria-valuemax="100"
				aria-valuenow={percent}
				aria-label={t('trickplay_admin.title')}
			>
				<div
					class="h-full bg-gold transition-[width] duration-700"
					style:width={`${total ? (status.ready / total) * 100 : 100}%`}
				></div>
				<div
					class="h-full bg-red-500/70 transition-[width] duration-700"
					style:width={`${total ? (status.failed / total) * 100 : 0}%`}
				></div>
			</div>

			<dl class="mt-4 grid grid-cols-3 gap-3 text-center">
				<div class="rounded-lg bg-black/30 p-3">
					<dt class="text-xs text-neutral-400">{t('trickplay_admin.ready')}</dt>
					<dd class="text-xl font-semibold text-gold tabular-nums">{status.ready}</dd>
				</div>
				<div class="rounded-lg bg-black/30 p-3">
					<dt class="text-xs text-neutral-400">{t('trickplay_admin.pending')}</dt>
					<dd class="text-xl font-semibold tabular-nums">{status.pending}</dd>
				</div>
				<div class="rounded-lg bg-black/30 p-3">
					<dt class="text-xs text-neutral-400">{t('trickplay_admin.failed')}</dt>
					<dd class="text-xl font-semibold tabular-nums {status.failed ? 'text-red-300' : ''}">
						{status.failed}
					</dd>
				</div>
			</dl>

			{#if status.current}
				<div class="mt-5 flex items-center gap-4 rounded-lg border border-gold/30 bg-gold/5 p-4">
					<Icon name="refresh" class="size-6 shrink-0 animate-spin text-gold [animation-duration:2.5s]" />
					<div class="min-w-0 flex-1">
						<p class="text-xs tracking-wide text-gold uppercase">{t('trickplay_admin.now')}</p>
						<p class="truncate font-medium">
							{itemLabel(status.current.title, status.current.season, status.current.episode)}
						</p>
					</div>
					<p class="shrink-0 text-sm text-neutral-300 tabular-nums">{formatTime(elapsed)}</p>
				</div>
			{:else if phase && phase !== 'working'}
				<p class="mt-5 text-sm text-neutral-400">{t(`trickplay_admin.explain_${phase}`)}</p>
			{/if}
			{#if status.average_seconds}
				<p class="mt-3 text-xs text-neutral-500">
					{t('trickplay_admin.pace', { seconds: Math.round(status.average_seconds) })}
				</p>
			{/if}
		</section>

		{#if status.libraries.length > 0}
			<section class="mt-8">
				<h2 class="mb-3 text-lg font-semibold">{t('trickplay_admin.libraries')}</h2>
				<ul class="flex flex-col gap-3">
					{#each status.libraries as library (library.library_id)}
						{@const done = percentDone(library.ready, library.failed, library.total)}
						<li class="rounded-lg border border-white/10 bg-surface/40 p-4">
							<div class="mb-2 flex items-baseline justify-between gap-3">
								<span class="truncate font-medium">{library.name}</span>
								<span class="shrink-0 text-sm text-neutral-400 tabular-nums">
									{library.total === 0
										? t('trickplay_admin.no_videos')
										: t('trickplay_admin.library_counts', {
												done: library.ready + library.failed,
												total: library.total,
												percent: done
											})}
								</span>
							</div>
							<div class="flex h-2 overflow-hidden rounded-full bg-white/10">
								<div
									class="h-full bg-gold transition-[width] duration-700"
									style:width={`${library.total ? (library.ready / library.total) * 100 : 0}%`}
								></div>
								<div
									class="h-full bg-red-500/70"
									style:width={`${library.total ? (library.failed / library.total) * 100 : 0}%`}
								></div>
							</div>
						</li>
					{/each}
				</ul>
			</section>
		{/if}

		{#if status.failures.length > 0}
			<section class="mt-8">
				<h2 class="mb-3 text-lg font-semibold">{t('trickplay_admin.failures')}</h2>
				<p class="mb-3 text-sm text-neutral-400">{t('trickplay_admin.failures_hint')}</p>
				<ul class="divide-y divide-white/10 rounded-lg border border-white/10 bg-surface/40">
					{#each status.failures as failure (failure.item_id)}
						<li class="flex flex-wrap items-center gap-3 p-4">
							<div class="min-w-0 flex-1">
								<p class="truncate font-medium">
									{itemLabel(failure.title, failure.season, failure.episode)}
								</p>
								<p class="truncate text-xs text-neutral-400" title={failure.error ?? ''}>
									{failure.library_name} · {failure.error ?? t('trickplay_admin.unknown_error')}
								</p>
							</div>
							<button
								type="button"
								disabled={retrying[failure.item_id]}
								onclick={() => retry(failure.item_id)}
								class="flex items-center gap-1.5 rounded-md border border-neutral-600 px-3 py-1.5 text-sm hover:border-gold focus-visible:outline-2 focus-visible:outline-gold disabled:opacity-50"
							>
								<Icon name="refresh" class="size-4" />
								{t('trickplay_admin.retry')}
							</button>
						</li>
					{/each}
				</ul>
			</section>
		{/if}
		{#if failed}
			<p class="mt-4 text-sm text-red-300" role="alert">{t('trickplay_admin.refresh_failed')}</p>
		{/if}
	{/if}
</main>
