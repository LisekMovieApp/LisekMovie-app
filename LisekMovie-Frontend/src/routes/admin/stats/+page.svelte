<!--
  Admin statistics (Faza 13): a dashboard over the playback sessions.

  Filters in one row (date range with presets, user, platform, device,
  content type, and a title picked from a ranking) drive one request to
  `GET /api/admin/stats/dashboard`; every card below reads that answer:
  totals, viewing per day and per hour, where viewers stop (drop-off by
  tenth of the title), breakdowns by platform, device and content type,
  rankings of users, films and series, and the session list with each
  session's stopping point. Clicking a title narrows everything to it, so
  "where do people give up on this film" is one click.

  Below: the bots' read-only key for `GET /api/stats/summary` (Discord /
  Telegram reports), and (Faza 4) playback quality per session.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { api, ApiError } from '$lib/playback/api';
	import type { components } from '$lib/api/schema';
	import { formatTime } from '$lib/player/format';
	import BarChart from '$lib/components/BarChart.svelte';
	import {
		CONTENT_TYPES,
		DEVICE_TYPES,
		PLATFORMS,
		dashboardQuery,
		dateInput,
		humanDuration,
		lastDays,
		percent,
		progressOf,
		type Dashboard,
		type SessionRow,
		type Slice,
		type StatsFilters,
		type TitleRank
	} from '$lib/admin/stats';

	type SessionQuality = components['schemas']['SessionQuality'];
	type KeyStatus = components['schemas']['StatsKeyStatus'];
	type NewKey = components['schemas']['NewStatsKey'];

	const timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
	const RANGES = [7, 30, 90, 365] as const;

	let token = $state('');
	let filters = $state<StatsFilters>({
		...lastDays(30),
		userId: '',
		platform: '',
		deviceType: '',
		content: '',
		item: ''
	});
	let itemTitle = $state('');
	let users = $state<{ id: string; username: string }[]>([]);
	let data = $state<Dashboard | null>(null);
	let sessions = $state<SessionRow[]>([]);
	let moreAvailable = $state(false);
	let loadingMore = $state(false);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let seq = 0;

	async function load(query: string) {
		const mine = ++seq;
		loading = true;
		error = null;
		try {
			const result = await api<Dashboard>(token, `/api/admin/stats/dashboard?${query}`);
			if (mine !== seq) return;
			data = result;
			sessions = result.sessions;
			moreAvailable = result.sessions.length === 50;
		} catch (err) {
			if (mine !== seq) return;
			error =
				err instanceof ApiError && err.code === 'invalid_range'
					? 'stats.invalid_range'
					: 'stats.failed';
		} finally {
			if (mine === seq) loading = false;
		}
	}

	$effect(() => {
		if (!token) return;
		const query = dashboardQuery(
			{
				from: filters.from,
				to: filters.to,
				userId: filters.userId,
				platform: filters.platform,
				deviceType: filters.deviceType,
				content: filters.content,
				item: filters.item
			},
			timeZone
		);
		if (query === null) {
			error = 'stats.invalid_range';
			return;
		}
		void load(query);
	});

	async function loadMore() {
		const query = dashboardQuery(filters, timeZone, { offset: String(sessions.length) });
		if (!query) return;
		loadingMore = true;
		try {
			const more = await api<SessionRow[]>(token, `/api/admin/stats/sessions?${query}`);
			sessions = [...sessions, ...more];
			moreAvailable = more.length === 50;
		} catch {
			moreAvailable = false;
		} finally {
			loadingMore = false;
		}
	}

	function setRange(days: number) {
		const range = lastDays(days);
		filters.from = range.from;
		filters.to = range.to;
	}

	function pickTitle(rank: TitleRank) {
		filters.item = rank.series_id ?? rank.id;
		itemTitle = rank.title;
	}

	function resetFilters() {
		filters = { ...lastDays(30), userId: '', platform: '', deviceType: '', content: '', item: '' };
		itemTitle = '';
	}

	// --- chart data -------------------------------------------------------------
	const dayFormat = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'numeric' });

	const dayBars = $derived.by(() => {
		if (!data) return [];
		const byDay = new Map(data.days.map((d) => [d.day, d]));
		const bars = [];
		const start = new Date(`${filters.from}T00:00:00`);
		const end = new Date(`${filters.to}T00:00:00`).getTime();
		for (let i = 0; i < 1100; i++) {
			const day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i);
			if (Number.isNaN(day.getTime()) || day.getTime() > end) break;
			const seconds = byDay.get(dateInput(day))?.watched_seconds ?? 0;
			bars.push({
				label: dayFormat.format(day),
				value: seconds,
				display: humanDuration(seconds)
			});
		}
		return bars;
	});

	const hourBars = $derived.by(() => {
		if (!data) return [];
		const byHour = new Map(data.hours.map((h) => [h.hour, h]));
		return Array.from({ length: 24 }, (_, hour) => {
			const seconds = byHour.get(hour)?.watched_seconds ?? 0;
			return {
				label: t('stats.hour_label', { hour }),
				tick: String(hour),
				value: seconds,
				display: humanDuration(seconds)
			};
		});
	});

	const dropoffBars = $derived(
		(data?.dropoff ?? []).map((count, i) => ({
			label: t('stats.dropoff_label', { from: i * 10, to: (i + 1) * 10 }),
			tick: `${i * 10}%`,
			value: count,
			display: String(count)
		}))
	);

	const finished = $derived((data?.totals.completed ?? 0) + (data?.totals.abandoned ?? 0));

	function sliceLabel(kind: 'platform' | 'device' | 'content', key: string): string {
		if (kind === 'platform') return t(`stats.platforms.${key}`);
		if (kind === 'device') return t(`stats.devices.${key}`);
		return t(key === 'series' ? 'stats.content_series' : 'stats.content_movie');
	}

	function sliceMax(slices: Slice[]): number {
		return Math.max(1, ...slices.map((s) => s.watched_seconds));
	}

	function episodeLabel(row: SessionRow): string {
		if (
			row.season === null ||
			row.season === undefined ||
			row.episode === null ||
			row.episode === undefined
		)
			return '';
		const code = t('stats.episode_label', { season: row.season, episode: row.episode });
		return row.episode_title ? `${code} · ${row.episode_title}` : code;
	}

	const started = (iso: string) =>
		new Date(iso).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });

	// --- bots' key --------------------------------------------------------------
	let keyStatus = $state<KeyStatus | null>(null);
	let newKey = $state<NewKey | null>(null);
	let keyBusy = $state(false);
	let keyFailed = $state(false);
	let copied = $state(false);

	async function loadKey() {
		try {
			keyStatus = await api<KeyStatus>(token, '/api/admin/stats/key');
		} catch {
			keyFailed = true;
		}
	}

	async function createKey() {
		keyBusy = true;
		keyFailed = false;
		copied = false;
		try {
			newKey = await api<NewKey>(token, '/api/admin/stats/key', 'POST');
			keyStatus = { configured: true, created_at: newKey.created_at };
		} catch {
			keyFailed = true;
		} finally {
			keyBusy = false;
		}
	}

	async function revokeKey() {
		keyBusy = true;
		keyFailed = false;
		try {
			await api(token, '/api/admin/stats/key', 'DELETE');
			keyStatus = { configured: false, created_at: null };
			newKey = null;
		} catch {
			keyFailed = true;
		} finally {
			keyBusy = false;
		}
	}

	async function copyKey() {
		if (!newKey) return;
		try {
			await navigator.clipboard.writeText(newKey.key);
			copied = true;
		} catch {
			copied = false;
		}
	}

	const example = $derived(
		`curl -H "X-Lisek-Stats-Key: ${newKey?.key ?? '<KEY>'}" "${typeof location === 'undefined' ? '' : location.origin}/api/stats/summary?period=weekly&format=discord"`
	);

	// --- playback quality (Faza 4) ----------------------------------------------
	let quality = $state<SessionQuality[]>([]);
	let qualityFailed = $state(false);
	const METHODS = new Set(['direct_play', 'remux', 'transcode']);

	async function loadQuality() {
		try {
			quality = await api<SessionQuality[]>(token, '/api/admin/stats/playback?limit=50');
			qualityFailed = false;
		} catch {
			qualityFailed = true;
		}
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		api<{ id: string; username: string }[]>(token, '/api/admin/users')
			.then((list) => (users = [...list].sort((a, b) => a.username.localeCompare(b.username))))
			.catch(() => (users = []));
		void loadKey();
		void loadQuality();
		const timer = setInterval(loadQuality, 15000);
		return () => clearInterval(timer);
	});

	const card = 'rounded-xl bg-white/[0.04] p-4 ring-1 ring-white/10 md:p-5';
	const field =
		'w-full rounded-md border border-white/15 bg-surface px-2.5 py-2 text-sm text-neutral-100 focus:border-gold focus:outline-none';
	const outcomeTone: Record<string, string> = {
		completed: 'bg-emerald-500/15 text-emerald-300 ring-emerald-400/30',
		watching: 'bg-sky-500/15 text-sky-300 ring-sky-400/30',
		abandoned: 'bg-amber-500/15 text-amber-300 ring-amber-400/30'
	};
</script>

<main class="mx-auto max-w-7xl px-4 py-8 text-white md:px-8">
	<h1 class="text-2xl font-semibold">{t('stats.title')}</h1>
	<p class="mt-1 mb-6 text-sm text-neutral-400">{t('stats.hint')}</p>

	<!-- Filters: one row, wrapping on small screens. -->
	<section class="{card} mb-6 flex flex-col gap-4" aria-label={t('stats.filters')}>
		<div class="flex flex-wrap items-center gap-2">
			{#each RANGES as days (days)}
				<button
					type="button"
					class="rounded-full border px-3 py-1.5 text-sm transition-colors focus-visible:outline-2 focus-visible:outline-gold {filters.to ===
						lastDays(days).to && filters.from === lastDays(days).from
						? 'border-gold bg-gold/10 text-gold'
						: 'border-white/15 text-neutral-300 hover:border-gold/60'}"
					onclick={() => setRange(days)}
				>
					{t(`stats.range_${days}`)}
				</button>
			{/each}
			{#if filters.item}
				<span
					class="flex items-center gap-1 rounded-full bg-gold/15 py-1 pr-1 pl-3 text-sm text-gold ring-1 ring-gold/40"
				>
					{t('stats.title_filter', { title: itemTitle })}
					<button
						type="button"
						class="grid size-6 place-items-center rounded-full hover:bg-gold/20 focus-visible:outline-2 focus-visible:outline-gold"
						aria-label={t('stats.clear_title')}
						onclick={() => {
							filters.item = '';
							itemTitle = '';
						}}
					>
						×
					</button>
				</span>
			{/if}
			<button
				type="button"
				class="ml-auto text-sm text-neutral-400 underline-offset-4 hover:text-gold hover:underline"
				onclick={resetFilters}
			>
				{t('stats.reset')}
			</button>
		</div>
		<div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
			<label class="text-xs text-neutral-400"
				>{t('stats.from')}<input
					class="{field} mt-1"
					type="date"
					bind:value={filters.from}
				/></label
			>
			<label class="text-xs text-neutral-400"
				>{t('stats.to')}<input class="{field} mt-1" type="date" bind:value={filters.to} /></label
			>
			<label class="text-xs text-neutral-400"
				>{t('stats.user')}<select class="{field} mt-1" bind:value={filters.userId}>
					<option value="">{t('stats.all_users')}</option>
					{#each users as user (user.id)}<option value={user.id}>{user.username}</option>{/each}
				</select></label
			>
			<label class="text-xs text-neutral-400"
				>{t('stats.platform')}<select class="{field} mt-1" bind:value={filters.platform}>
					<option value="">{t('stats.all_platforms')}</option>
					{#each PLATFORMS as platform (platform)}<option value={platform}
							>{t(`stats.platforms.${platform}`)}</option
						>{/each}
				</select></label
			>
			<label class="text-xs text-neutral-400"
				>{t('stats.device')}<select class="{field} mt-1" bind:value={filters.deviceType}>
					<option value="">{t('stats.all_devices')}</option>
					{#each DEVICE_TYPES as device (device)}<option value={device}
							>{t(`stats.devices.${device}`)}</option
						>{/each}
				</select></label
			>
			<label class="text-xs text-neutral-400"
				>{t('stats.content')}<select class="{field} mt-1" bind:value={filters.content}>
					<option value="">{t('stats.all_content')}</option>
					{#each CONTENT_TYPES as content (content)}<option value={content}
							>{sliceLabel('content', content)}</option
						>{/each}
				</select></label
			>
		</div>
	</section>

	{#if error}
		<p role="alert" class="mb-6 text-sm text-red-400">{t(error)}</p>
	{/if}

	{#if data}
		<div class="transition-opacity {loading ? 'opacity-60' : ''}" aria-busy={loading}>
			<!-- Totals -->
			<section class="mb-6 grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-6">
				{#each [{ key: 'stats.watch_time', value: humanDuration(data.totals.watched_seconds) }, { key: 'stats.plays', value: String(data.totals.plays) }, { key: 'stats.active_users', value: String(data.totals.users) }, { key: 'stats.titles', value: String(data.totals.titles) }, { key: 'stats.completion', value: `${percent(data.totals.completed, finished)}%` }, { key: 'stats.avg_play', value: humanDuration(data.totals.plays > 0 ? data.totals.watched_seconds / data.totals.plays : 0) }] as tile (tile.key)}
					<div class={card}>
						<p class="text-xs text-neutral-400">{t(tile.key)}</p>
						<p class="mt-1 text-2xl font-semibold tabular-nums">{tile.value}</p>
					</div>
				{/each}
			</section>

			{#if data.totals.sessions === 0}
				<p class="mb-6 text-neutral-400">{t('stats.empty')}</p>
			{:else}
				<section class="{card} mb-6">
					<h2 class="mb-4 font-semibold">{t('stats.per_day')}</h2>
					<BarChart bars={dayBars} label={t('stats.per_day')} valueLabel={t('stats.watch_time')} />
				</section>

				<div class="mb-6 grid gap-6 lg:grid-cols-2">
					<section class={card}>
						<h2 class="mb-4 font-semibold">{t('stats.per_hour')}</h2>
						<BarChart
							bars={hourBars}
							label={t('stats.per_hour')}
							valueLabel={t('stats.watch_time')}
							maxTicks={6}
						/>
					</section>
					<section class={card}>
						<h2 class="font-semibold">{t('stats.dropoff')}</h2>
						<p class="mt-1 mb-4 text-xs text-neutral-400">
							{t('stats.dropoff_hint', {
								completed: data.totals.completed,
								abandoned: data.totals.abandoned
							})}
						</p>
						<BarChart
							bars={dropoffBars}
							label={t('stats.dropoff')}
							valueLabel={t('stats.sessions_count')}
							maxTicks={5}
						/>
					</section>
				</div>

				<!-- Breakdowns -->
				<div class="mb-6 grid gap-6 md:grid-cols-3">
					{#each [{ title: 'stats.by_platform', kind: 'platform', slices: data.platforms }, { title: 'stats.by_device', kind: 'device', slices: data.device_types }, { title: 'stats.by_content', kind: 'content', slices: data.content }] as group (group.title)}
						<section class={card}>
							<h2 class="mb-3 font-semibold">{t(group.title)}</h2>
							<ul class="flex flex-col gap-2.5">
								{#each group.slices as slice (slice.key)}
									{@const kind = group.kind as 'platform' | 'device' | 'content'}
									<li>
										<button
											type="button"
											class="group w-full text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gold"
											onclick={() => {
												if (kind === 'platform') filters.platform = slice.key;
												else if (kind === 'device') filters.deviceType = slice.key;
												else filters.content = slice.key;
											}}
										>
											<span class="flex items-baseline justify-between gap-2 text-sm">
												<span class="truncate group-hover:text-gold"
													>{sliceLabel(kind, slice.key)}</span
												>
												<span class="shrink-0 text-xs text-neutral-400 tabular-nums"
													>{humanDuration(slice.watched_seconds)} · {slice.plays}</span
												>
											</span>
											<span class="mt-1 block h-1.5 overflow-hidden rounded-full bg-white/10">
												<span
													class="block h-full rounded-full bg-gold"
													style:width={`${(slice.watched_seconds / sliceMax(group.slices)) * 100}%`}
												></span>
											</span>
										</button>
									</li>
								{/each}
							</ul>
						</section>
					{/each}
				</div>

				<!-- Rankings -->
				<div class="mb-6 grid gap-6 xl:grid-cols-3">
					<section class={card}>
						<h2 class="mb-3 font-semibold">{t('stats.top_users')}</h2>
						<table class="w-full text-sm">
							<thead class="text-left text-xs text-neutral-400">
								<tr>
									<th class="py-1.5 font-normal">{t('stats.col_user')}</th>
									<th class="py-1.5 text-right font-normal">{t('stats.col_time')}</th>
									<th class="py-1.5 text-right font-normal">{t('stats.col_plays')}</th>
								</tr>
							</thead>
							<tbody>
								{#each data.users as user (user.user_id)}
									<tr class="border-t border-white/10">
										<td class="py-2">
											<button
												type="button"
												class="truncate text-left hover:text-gold focus-visible:outline-2 focus-visible:outline-gold"
												onclick={() => (filters.userId = user.user_id)}>{user.username}</button
											>
										</td>
										<td class="py-2 text-right tabular-nums"
											>{humanDuration(user.watched_seconds)}</td
										>
										<td class="py-2 text-right tabular-nums">{user.plays}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</section>
					{#each [{ title: 'stats.top_movies', rows: data.top_movies }, { title: 'stats.top_series', rows: data.top_series }] as ranking (ranking.title)}
						<section class={card}>
							<h2 class="mb-3 font-semibold">{t(ranking.title)}</h2>
							{#if ranking.rows.length === 0}
								<p class="text-sm text-neutral-500">{t('stats.empty_short')}</p>
							{:else}
								<table class="w-full table-fixed text-sm">
									<thead class="text-left text-xs text-neutral-400">
										<tr>
											<th class="py-1.5 font-normal">{t('stats.col_title')}</th>
											<th class="w-20 py-1.5 text-right font-normal">{t('stats.col_time')}</th>
											<th class="w-14 py-1.5 text-right font-normal" title={t('stats.col_plays')}
												>{t('stats.col_plays_short')}</th
											>
											<th class="w-14 py-1.5 text-right font-normal" title={t('stats.col_viewers')}
												>{t('stats.col_viewers_short')}</th
											>
										</tr>
									</thead>
									<tbody>
										{#each ranking.rows as row, i (row.series_id ?? row.id)}
											<tr class="border-t border-white/10">
												<td class="py-2">
													<span class="flex min-w-0 items-center gap-2">
														<span class="w-4 shrink-0 text-xs text-neutral-500 tabular-nums"
															>{i + 1}</span
														>
														<button
															type="button"
															class="truncate text-left hover:text-gold focus-visible:outline-2 focus-visible:outline-gold"
															title={t('stats.filter_by_title', { title: row.title })}
															onclick={() => pickTitle(row)}>{row.title}</button
														>
														<a
															href={resolve('/media/[id]', { id: row.id })}
															class="shrink-0 text-xs text-neutral-500 hover:text-gold"
															aria-label={t('stats.open_title', { title: row.title })}>↗</a
														>
													</span>
												</td>
												<td class="py-2 text-right tabular-nums"
													>{humanDuration(row.watched_seconds)}</td
												>
												<td class="py-2 text-right tabular-nums">{row.plays}</td>
												<td class="py-2 text-right tabular-nums">{row.users}</td>
											</tr>
										{/each}
									</tbody>
								</table>
							{/if}
						</section>
					{/each}
				</div>

				<!-- Sessions: where each viewer stopped. -->
				<section class="{card} mb-6">
					<h2 class="font-semibold">{t('stats.sessions')}</h2>
					<p class="mt-1 mb-4 text-xs text-neutral-400">{t('stats.sessions_hint')}</p>
					<div class="overflow-x-auto">
						<table class="w-full min-w-[52rem] text-left text-sm">
							<thead class="text-xs text-neutral-400">
								<tr>
									<th class="py-2 pr-3 font-normal">{t('stats.col_started')}</th>
									<th class="py-2 pr-3 font-normal">{t('stats.col_user')}</th>
									<th class="py-2 pr-3 font-normal">{t('stats.col_title')}</th>
									<th class="py-2 pr-3 font-normal">{t('stats.col_device')}</th>
									<th class="py-2 pr-3 text-right font-normal">{t('stats.col_time')}</th>
									<th class="w-56 py-2 font-normal">{t('stats.col_progress')}</th>
								</tr>
							</thead>
							<tbody>
								{#each sessions as row (row.id)}
									{@const progress = progressOf(row)}
									<tr class="border-t border-white/10 align-top">
										<td class="py-2.5 pr-3 whitespace-nowrap text-neutral-300"
											>{started(row.started_at)}</td
										>
										<td class="py-2.5 pr-3">{row.username}</td>
										<td class="max-w-72 py-2.5 pr-3">
											<span class="block truncate">{row.title}</span>
											{#if episodeLabel(row)}
												<span class="block truncate text-xs text-neutral-400"
													>{episodeLabel(row)}</span
												>
											{/if}
										</td>
										<td class="py-2.5 pr-3 whitespace-nowrap text-neutral-300">
											{t(`stats.platforms.${row.platform}`)}
											<span class="block text-xs text-neutral-500"
												>{t(`stats.devices.${row.device_type}`)}{row.browser
													? ` · ${row.browser}`
													: ''}</span
											>
										</td>
										<td class="py-2.5 pr-3 text-right whitespace-nowrap tabular-nums"
											>{humanDuration(row.watched_seconds)}</td
										>
										<td class="py-2.5">
											<span class="flex items-center justify-between gap-2">
												<span
													class="rounded-full px-2 py-0.5 text-xs ring-1 {outcomeTone[
														row.outcome
													] ?? 'bg-white/10 text-neutral-300 ring-white/20'}"
													>{t(`stats.outcomes.${row.outcome}`)}</span
												>
												<span class="text-xs text-neutral-400 tabular-nums">
													{progress === null
														? t('stats.no_length')
														: t('stats.stopped_at', {
																time: formatTime(row.position_seconds),
																percent: Math.round(progress * 100)
															})}
												</span>
											</span>
											<span
												class="relative mt-1.5 block h-1.5 rounded-full bg-white/10"
												role="presentation"
											>
												{#if progress !== null}
													<span
														class="absolute inset-y-0 left-0 rounded-full {row.outcome ===
														'abandoned'
															? 'bg-amber-400'
															: 'bg-gold'}"
														style:width={`${progress * 100}%`}
													></span>
													<span
														class="absolute top-1/2 size-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white ring-2 ring-black"
														style:left={`${progress * 100}%`}
													></span>
												{/if}
											</span>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
					{#if moreAvailable}
						<button
							type="button"
							class="mt-4 rounded-md border border-white/15 px-4 py-2 text-sm hover:border-gold focus-visible:outline-2 focus-visible:outline-gold disabled:opacity-50"
							disabled={loadingMore}
							onclick={loadMore}
						>
							{t('stats.load_more')}
						</button>
					{/if}
				</section>
			{/if}
		</div>
	{:else if loading && !error}
		<p role="status" class="mb-6 text-neutral-400">{t('stats.loading')}</p>
	{/if}

	<!-- Bots' key -->
	<section class="{card} mb-10">
		<h2 class="font-semibold">{t('stats.bot_title')}</h2>
		<p class="mt-1 mb-4 text-sm text-neutral-400">{t('stats.bot_hint')}</p>
		{#if keyStatus}
			<p class="mb-3 text-sm">
				{keyStatus.configured && keyStatus.created_at
					? t('stats.bot_configured', { date: started(keyStatus.created_at) })
					: t('stats.bot_none')}
			</p>
		{/if}
		<div class="flex flex-wrap gap-2">
			<button
				type="button"
				class="rounded-md bg-gold px-4 py-2 text-sm font-medium text-black hover:bg-[#d6bb66] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gold disabled:opacity-50"
				disabled={keyBusy}
				onclick={createKey}
			>
				{keyStatus?.configured ? t('stats.bot_rotate') : t('stats.bot_create')}
			</button>
			{#if keyStatus?.configured}
				<button
					type="button"
					class="rounded-md border border-white/15 px-4 py-2 text-sm hover:border-red-400 hover:text-red-300 focus-visible:outline-2 focus-visible:outline-gold disabled:opacity-50"
					disabled={keyBusy}
					onclick={revokeKey}
				>
					{t('stats.bot_revoke')}
				</button>
			{/if}
		</div>
		{#if newKey}
			<div class="mt-4 rounded-lg bg-black/40 p-3 ring-1 ring-gold/40">
				<p class="mb-2 text-sm text-gold">{t('stats.bot_new')}</p>
				<div class="flex flex-wrap items-center gap-2">
					<code class="min-w-0 flex-1 rounded bg-white/5 px-2 py-1.5 text-xs break-all"
						>{newKey.key}</code
					>
					<button
						type="button"
						class="rounded-md border border-white/15 px-3 py-1.5 text-sm hover:border-gold focus-visible:outline-2 focus-visible:outline-gold"
						onclick={copyKey}
					>
						{copied ? t('stats.bot_copied') : t('stats.bot_copy')}
					</button>
				</div>
			</div>
		{/if}
		<p class="mt-4 mb-1 text-xs text-neutral-400">{t('stats.bot_example')}</p>
		<pre class="overflow-x-auto rounded bg-black/40 p-3 text-xs text-neutral-300">{example}</pre>
		{#if keyFailed}<p role="alert" class="mt-3 text-sm text-red-400">
				{t('stats.bot_failed')}
			</p>{/if}
	</section>

	<!-- Playback quality (Faza 4) -->
	<h2 class="mb-2 text-xl font-semibold">{t('player.quality_title')}</h2>
	<p class="mb-4 text-sm text-neutral-400">{t('player.quality_hint')}</p>
	{#if qualityFailed}
		<p role="alert">{t('player.load_failed')}</p>
	{:else if quality.length === 0}
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
					{#each quality as row (row.id)}
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
