<!--
  The transcoding pool (Faza 14), under the hardware diagnostics: every
  GPU the server knows (saved pool + latest diagnostics) as a card with
  - a switch putting it in or out of the pool;
  - its API (Intel/AMD render nodes: Quick Sync or VA-API);
  - its share of sessions and a session limit;
  - the source codecs it decodes and the output codecs it encodes, each
    marked with the diagnostics' test result;
  - LisekMovie's transcodes on it right now (refreshed every 10 s).
  A preview shows how simultaneous sessions would spread (the server's
  rule). "Save" stores the pool and, with "Transcode with the pool" on,
  makes it the hardware acceleration mode; `onsaved` lets the settings
  form below reload.
-->
<script lang="ts">
	import { onMount, untrack } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { api } from '$lib/playback/api';
	import type { components } from '$lib/api/schema';
	import {
		DECODE_CODECS,
		ENCODE_CODECS,
		activeDevices,
		deviceId,
		mergePool,
		simulate,
		switchableApi,
		testResult,
		type PoolDevice,
		type Report
	} from '$lib/admin/pool';

	type Status = components['schemas']['HardwareStatus'];
	type Balancing = components['schemas']['BalancingInfo'];

	interface Props {
		/** The latest diagnostics (suggestions and test results). */
		report: Report | null;
		onsaved?: () => void;
	}

	let { report, onsaved }: Props = $props();

	let token = '';
	let devices = $state<PoolDevice[]>([]);
	let usePool = $state(false);
	let balancing = $state<Balancing | null>(null);
	let loaded = $state(false);
	let dirty = $state(false);
	let saving = $state(false);
	let message = $state<'saved' | 'failed' | null>(null);
	let previewSessions = $state(3);

	async function load() {
		try {
			const status = await api<Status>(token, '/api/admin/hardware');
			balancing = status.balancing;
			if (!dirty) {
				devices = mergePool(status.pool ?? [], status.report?.pool_candidates ?? []);
				usePool = status.balancing.mode === 'pool';
			}
			loaded = true;
		} catch {
			// Keeps the last values.
		}
	}

	// New diagnostics: add the devices they found (saved ones stay as they are).
	$effect(() => {
		const candidates = report?.pool_candidates;
		if (!candidates || !loaded) return;
		untrack(() => {
			devices = mergePool(devices, candidates);
		});
	});

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		void load();
		const timer = setInterval(load, 10000);
		return () => clearInterval(timer);
	});

	function edit(index: number, change: Partial<PoolDevice>) {
		devices[index] = { ...devices[index], ...change };
		dirty = true;
		message = null;
	}

	function toggleCodec(index: number, field: 'decode_codecs' | 'encode_codecs', codec: string) {
		const list = devices[index][field];
		edit(index, {
			[field]: list.includes(codec) ? list.filter((c) => c !== codec) : [...list, codec]
		});
	}

	function switchApi(index: number) {
		const d = devices[index];
		const api = d.api === 'qsv' ? 'vaapi' : 'qsv';
		if (devices.some((other, i) => i !== index && deviceId(other) === `${api}:${d.device}`)) return;
		edit(index, { api });
	}

	async function save() {
		saving = true;
		message = null;
		try {
			const status = await api<Status>(token, '/api/admin/hardware/pool', 'PUT', {
				devices,
				use_pool: usePool
			});
			balancing = status.balancing;
			dirty = false;
			message = 'saved';
			onsaved?.();
		} catch {
			message = 'failed';
		} finally {
			saving = false;
		}
	}

	const load_of = (d: PoolDevice) =>
		balancing?.devices.find((l) => l.id === deviceId(d))?.transcodes ?? 0;
	const preview = $derived(simulate(devices, previewSessions));
	const anyActive = $derived(activeDevices(devices).length > 0);

	const card = 'rounded-xl bg-white/[0.04] p-4 ring-1 ring-white/10';
	const apiTone: Record<string, string> = {
		nvenc: 'bg-emerald-500/15 text-emerald-300 ring-emerald-400/30',
		qsv: 'bg-sky-500/15 text-sky-300 ring-sky-400/30',
		vaapi: 'bg-violet-500/15 text-violet-300 ring-violet-400/30'
	};
	const codecChip =
		'rounded-full px-2.5 py-1 text-xs ring-1 transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gold';
</script>

<section class="mt-6" aria-labelledby="pool-title">
	<div class="flex flex-wrap items-end justify-between gap-3">
		<div>
			<h2 id="pool-title" class="text-xl font-semibold">{t('hardware.pool_title')}</h2>
			<p class="mt-1 max-w-2xl text-sm text-neutral-400">{t('hardware.pool_hint')}</p>
		</div>
		{#if balancing}
			<span class="rounded-full bg-white/10 px-3 py-1 text-xs text-neutral-300"
				>{t('hardware.mode', { mode: t(`hardware.modes.${balancing.mode}`) })}</span
			>
		{/if}
	</div>

	{#if loaded && devices.length === 0}
		<p class="{card} mt-4 text-sm text-neutral-400">{t('hardware.pool_empty')}</p>
	{/if}

	<div class="mt-4 grid gap-4 lg:grid-cols-2">
		{#each devices as device, i (deviceId(device) + i)}
			<article
				class="{card} flex flex-col gap-4 transition-opacity {device.enabled ? '' : 'opacity-70'}"
				aria-label={device.name || deviceId(device)}
			>
				<header class="flex items-start gap-3">
					<div class="min-w-0 flex-1">
						<div class="flex flex-wrap items-center gap-2">
							<span
								class="rounded-full px-2 py-0.5 text-xs font-medium ring-1 {apiTone[device.api] ??
									''}">{t(`hardware.apis.${device.api}`)}</span
							>
							<span class="font-mono text-xs text-neutral-400">{device.device}</span>
						</div>
						<p class="mt-1.5 truncate font-semibold">{device.name || deviceId(device)}</p>
						<p class="mt-0.5 text-xs text-neutral-400">
							{t('hardware.lisek_transcodes', { count: load_of(device) })}
						</p>
					</div>
					<label class="flex shrink-0 cursor-pointer items-center gap-2 text-sm">
						<span class="sr-only">{t('hardware.in_pool')}</span>
						<input
							type="checkbox"
							role="switch"
							class="peer sr-only"
							checked={device.enabled}
							onchange={(e) => edit(i, { enabled: (e.currentTarget as HTMLInputElement).checked })}
						/>
						<span
							class="relative h-6 w-11 rounded-full bg-white/15 transition-colors peer-checked:bg-gold peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-gold after:absolute after:top-0.5 after:left-0.5 after:size-5 after:rounded-full after:bg-white after:transition-transform peer-checked:after:translate-x-5"
							aria-hidden="true"
						></span>
					</label>
				</header>

				<div class="grid grid-cols-2 items-end gap-3 text-xs text-neutral-400">
					<label
						>{t('hardware.weight')}
						<select
							class="mt-1 w-full rounded-md border border-white/15 bg-surface px-2 py-1.5 text-sm text-neutral-100"
							value={device.weight}
							onchange={(e) =>
								edit(i, { weight: Number((e.currentTarget as HTMLSelectElement).value) })}
						>
							{#each [1, 2, 3, 4, 6, 8] as w (w)}<option value={w}
									>{t('hardware.weight_n', { n: w })}</option
								>{/each}
						</select>
					</label>
					<label
						>{t('hardware.max_sessions')}
						<input
							type="number"
							min="0"
							max="64"
							class="mt-1 w-full rounded-md border border-white/15 bg-surface px-2 py-1.5 text-sm text-neutral-100"
							value={device.max_sessions}
							onchange={(e) =>
								edit(i, {
									max_sessions: Math.min(
										64,
										Math.max(0, Number((e.currentTarget as HTMLInputElement).value) || 0)
									)
								})}
						/>
					</label>
				</div>

				{#if switchableApi(device)}
					<div class="flex items-center justify-between gap-2 text-xs text-neutral-400">
						<span>{t('hardware.api_choice')}</span>
						<button
							type="button"
							class="rounded-md border border-white/15 px-2.5 py-1 text-neutral-200 hover:border-gold focus-visible:outline-2 focus-visible:outline-gold"
							onclick={() => switchApi(i)}
						>
							{t(device.api === 'qsv' ? 'hardware.use_vaapi' : 'hardware.use_qsv')}
						</button>
					</div>
				{/if}

				<fieldset>
					<legend class="mb-1.5 text-xs text-neutral-400">{t('hardware.decodes')}</legend>
					<div class="flex flex-wrap gap-1.5">
						{#each DECODE_CODECS as codec (codec)}
							{@const on = device.decode_codecs.includes(codec)}
							<button
								type="button"
								aria-pressed={on}
								class="{codecChip} {on
									? 'bg-gold/15 text-gold ring-gold/50'
									: 'bg-transparent text-neutral-400 ring-white/15 hover:text-neutral-200'}"
								onclick={() => toggleCodec(i, 'decode_codecs', codec)}
								>{t(`hardware.codecs.${codec}`)}</button
							>
						{/each}
					</div>
				</fieldset>

				<fieldset>
					<legend class="mb-1.5 text-xs text-neutral-400">{t('hardware.encodes')}</legend>
					<div class="flex flex-wrap gap-1.5">
						{#each ENCODE_CODECS as codec (codec)}
							{@const on = device.encode_codecs.includes(codec)}
							{@const tested = report ? testResult(report.encoders, device, codec) : null}
							<button
								type="button"
								aria-pressed={on}
								title={tested === null
									? t('hardware.untested')
									: tested
										? t('hardware.tested_ok')
										: t('hardware.tested_failed')}
								class="{codecChip} {on
									? 'bg-gold/15 text-gold ring-gold/50'
									: 'bg-transparent text-neutral-400 ring-white/15 hover:text-neutral-200'}"
								onclick={() => toggleCodec(i, 'encode_codecs', codec)}
							>
								{t(`hardware.codecs.${codec}`)}
								{#if tested === true}<span class="ml-1 text-emerald-300">✓</span
									>{:else if tested === false}<span class="ml-1 text-red-300">✗</span>{/if}
							</button>
						{/each}
					</div>
					{#if device.enabled && device.encode_codecs.some((c) => report && testResult(report.encoders, device, c) === false)}
						<p class="mt-2 text-xs text-amber-300">{t('hardware.failed_codec_warning')}</p>
					{/if}
				</fieldset>
			</article>
		{/each}
	</div>

	{#if devices.length > 0}
		<div class="{card} mt-4 flex flex-col gap-4">
			<label class="flex cursor-pointer items-start gap-3 text-sm">
				<input
					type="checkbox"
					class="mt-0.5 size-4 accent-gold"
					bind:checked={usePool}
					onchange={() => (dirty = true)}
				/>
				<span>
					{t('hardware.use_pool')}
					<span class="block text-xs text-neutral-400">{t('hardware.use_pool_hint')}</span>
				</span>
			</label>

			{#if anyActive}
				<div>
					<p class="text-xs text-neutral-400">
						{t('hardware.preview', { sessions: previewSessions })}
					</p>
					<input
						type="range"
						min="1"
						max="12"
						class="mt-1 w-full max-w-xs accent-gold"
						aria-label={t('hardware.preview_sessions')}
						bind:value={previewSessions}
					/>
					<ul class="mt-2 flex flex-wrap gap-2 text-sm">
						{#each activeDevices(devices) as d (deviceId(d))}
							<li class="rounded-lg bg-black/30 px-3 py-1.5 ring-1 ring-white/10">
								{d.name || deviceId(d)}
								<span class="ml-1.5 font-semibold text-gold tabular-nums"
									>{preview.counts.get(deviceId(d)) ?? 0}</span
								>
							</li>
						{/each}
						{#if preview.cpu > 0}
							<li class="rounded-lg bg-black/30 px-3 py-1.5 ring-1 ring-white/10">
								{t('hardware.cpu_fallback')}
								<span class="ml-1.5 font-semibold text-neutral-300 tabular-nums">{preview.cpu}</span
								>
							</li>
						{/if}
					</ul>
				</div>
			{/if}

			<div class="flex flex-wrap items-center gap-3">
				<button
					type="button"
					class="rounded-md bg-gold px-4 py-2.5 font-medium text-black hover:bg-[#d6bb66] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gold disabled:opacity-50"
					disabled={saving || !dirty}
					onclick={save}
				>
					{saving ? t('hardware.saving') : t('hardware.save_pool')}
				</button>
				{#if message === 'saved'}<span role="status" class="text-sm text-gold"
						>{t('hardware.pool_saved')}</span
					>{/if}
				{#if message === 'failed'}<span role="alert" class="text-sm text-red-400"
						>{t('hardware.pool_failed')}</span
					>{/if}
				{#if usePool && !anyActive}<span class="text-sm text-amber-300"
						>{t('hardware.pool_none_active')}</span
					>{/if}
			</div>
		</div>
	{/if}

	{#if balancing && balancing.devices.length > 0}
		<div class="{card} mt-4">
			<h3 class="text-sm font-semibold">{t('hardware.live_load')}</h3>
			<ul class="mt-2 flex flex-wrap gap-2">
				{#each balancing.devices as load (load.id)}
					<li class="rounded-lg bg-black/30 px-3 py-2 text-sm ring-1 ring-white/10">
						{load.name || load.id}
						<span class="ml-2 font-semibold text-gold tabular-nums"
							>{load.max_sessions > 0
								? t('hardware.load_of', { count: load.transcodes, max: load.max_sessions })
								: t('hardware.transcodes', { count: load.transcodes })}</span
						>
					</li>
				{/each}
			</ul>
		</div>
	{/if}
</section>
