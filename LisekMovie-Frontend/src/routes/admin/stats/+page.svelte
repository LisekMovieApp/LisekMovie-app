<!--
  Admin statistics: viewing time per user, and (Faza 4) playback quality
  per session from the player's telemetry (`GET /api/admin/stats/playback`):
  method, rebuffering, dropped frames and bandwidth show which clients or
  play methods struggle. The quality table refreshes every 15 s.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { api } from '$lib/playback/api';
	import type { components } from '$lib/api/schema';
	import { formatTime } from '$lib/player/format';

	type SessionQuality = components['schemas']['SessionQuality'];

	let rows = $state<{ username: string; plays: number; watched_seconds: number }[]>([]);
	let sessions = $state<SessionQuality[]>([]);
	let loading = $state(true);
	let failed = $state(false);
	let qualityFailed = $state(false);

	const METHODS = new Set(['direct_play', 'remux', 'transcode']);

	async function loadQuality(token: string) {
		try {
			sessions = await api<SessionQuality[]>(token, '/api/admin/stats/playback?limit=50');
			qualityFailed = false;
		} catch {
			qualityFailed = true;
		}
	}

	onMount(() => {
		const token = getSessionCookieClientSide() ?? '';
		api<typeof rows>(token, '/api/admin/stats/users')
			.then((data) => (rows = data))
			.catch(() => (failed = true))
			.finally(() => (loading = false));
		void loadQuality(token);
		const timer = setInterval(() => loadQuality(token), 15000);
		return () => clearInterval(timer);
	});

	const started = (iso: string) =>
		new Date(iso).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });
</script>

<main class="mx-auto max-w-6xl px-4 py-8 text-white">
	<h1 class="mb-4 text-2xl font-semibold">{t('player.stats')}</h1>
	<p class="mb-6 text-sm text-neutral-400">{t('player.stats_hint')}</p>
	{#if loading}<p>{t('common.loading')}</p>
	{:else if failed}<p role="alert">{t('player.load_failed')}</p>
	{:else if rows.length === 0}<p>{t('player.list_empty')}</p>
	{:else}
		<table class="w-full max-w-3xl text-left">
			<thead>
				<tr class="border-b border-white/20">
					<th class="p-3">{t('player.user')}</th>
					<th class="p-3">{t('player.plays')}</th>
					<th class="p-3">{t('player.minutes')}</th>
				</tr>
			</thead>
			<tbody>
				{#each rows as row (row.username)}
					<tr class="border-b border-white/10">
						<td class="p-3">{row.username}</td>
						<td class="p-3">{row.plays}</td>
						<td class="p-3">{Math.round(row.watched_seconds / 60)}</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}

	<h2 class="mt-12 mb-2 text-xl font-semibold">{t('player.quality_title')}</h2>
	<p class="mb-4 text-sm text-neutral-400">{t('player.quality_hint')}</p>
	{#if qualityFailed}
		<p role="alert">{t('player.load_failed')}</p>
	{:else if sessions.length === 0}
		<p class="text-neutral-400">{t('player.list_empty')}</p>
	{:else}
		<div class="overflow-x-auto rounded-lg ring-1 ring-white/10">
			<table class="w-full text-left text-sm whitespace-nowrap">
				<thead class="bg-white/5 text-neutral-300">
					<tr>
						<th class="p-3">{t('player.quality_started')}</th>
						<th class="p-3">{t('player.user')}</th>
						<th class="p-3">{t('player.quality_item')}</th>
						<th class="p-3">{t('player.diag_mode')}</th>
						<th class="p-3">{t('player.quality_client')}</th>
						<th class="p-3 text-right">{t('player.quality_position')}</th>
						<th class="p-3 text-right">{t('player.minutes')}</th>
						<th class="p-3 text-right">{t('player.quality_stalls')}</th>
						<th class="p-3 text-right">{t('player.diag_dropped')}</th>
						<th class="p-3 text-right">{t('player.diag_bandwidth')}</th>
					</tr>
				</thead>
				<tbody>
					{#each sessions as row (row.id)}
						<tr class="border-t border-white/10">
							<td class="p-3">
								<span class="flex items-center gap-2">
									<span
										class="size-2 rounded-full {row.active ? 'bg-emerald-400' : 'bg-neutral-600'}"
										title={row.active ? t('player.quality_active') : t('player.quality_ended')}
									></span>
									{started(row.started_at)}
								</span>
							</td>
							<td class="p-3">{row.username}</td>
							<td class="max-w-64 truncate p-3">{row.title}</td>
							<td class="p-3">
								{METHODS.has(row.play_method)
									? t(`player.modes.${row.play_method}`)
									: row.play_method}
							</td>
							<td class="max-w-56 truncate p-3 text-neutral-300">
								{[row.client_name, row.device_name].filter(Boolean).join(' · ')}
							</td>
							<td class="p-3 text-right tabular-nums">{formatTime(row.position_seconds)}</td>
							<td class="p-3 text-right tabular-nums">{Math.round(row.watched_seconds / 60)}</td>
							<td class="p-3 text-right tabular-nums {row.stall_count > 3 ? 'text-gold' : ''}">
								{row.stall_count} / {t('player.seconds', { value: row.stall_seconds.toFixed(1) })}
							</td>
							<td class="p-3 text-right tabular-nums">{row.dropped_frames ?? '–'}</td>
							<td class="p-3 text-right tabular-nums">
								{row.bandwidth_kbps
									? t('player.mbps', { value: (row.bandwidth_kbps / 1000).toFixed(1) })
									: '–'}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
	{/if}
</main>
