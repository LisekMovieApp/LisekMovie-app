<!--
  Hardware diagnostics (Faza 13), at the top of "Sprzęt i Transkodowanie".
  The button runs `POST /api/admin/hardware/diagnostics` (a few seconds):
  CPU, GPUs (NVIDIA via nvidia-smi, Intel/AMD render nodes, WMI on
  Windows), VA-API profiles, Vulkan devices, the FFmpeg build and one
  test encode per hardware encoder and device. Faza 14: below the button,
  the transcoding pool editor (`TranscodePool`) turns the result into
  devices the administrator switches on and assigns codecs to.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { api } from '$lib/playback/api';
	import type { components } from '$lib/api/schema';
	import TranscodePool from './TranscodePool.svelte';

	type Report = components['schemas']['HardwareReport'];
	type Status = components['schemas']['HardwareStatus'];

	interface Props {
		/** The pool was saved (the settings form reloads). */
		onsaved?: () => void;
	}

	let { onsaved }: Props = $props();
	let token = '';
	let report = $state<Report | null>(null);
	let running = $state(false);
	let failed = $state(false);

	async function refresh() {
		try {
			const status = await api<Status>(token, '/api/admin/hardware');
			report ??= status.report ?? null;
		} catch {
			// The card keeps the last values.
		}
	}

	async function diagnose() {
		running = true;
		failed = false;
		try {
			report = await api<Report>(token, '/api/admin/hardware/diagnostics', 'POST');
		} catch {
			failed = true;
		} finally {
			running = false;
		}
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		void refresh();
	});

	const mb = (value: number | null | undefined) =>
		value === null || value === undefined
			? '–'
			: value >= 1024
				? t('hardware.gb', { value: (value / 1024).toFixed(1) })
				: t('hardware.mb', { value });

	const card = 'rounded-xl bg-white/[0.04] p-4 ring-1 ring-white/10';
	const chip = 'rounded-full bg-white/10 px-2 py-0.5 text-xs text-neutral-200';
	const vendorTone: Record<string, string> = {
		nvidia: 'bg-emerald-500/15 text-emerald-300 ring-emerald-400/30',
		intel: 'bg-sky-500/15 text-sky-300 ring-sky-400/30',
		amd: 'bg-red-500/15 text-red-300 ring-red-400/30'
	};
</script>

<section class="mb-12" aria-labelledby="hardware-title">
	<h1 id="hardware-title" class="mb-2 text-2xl">{t('hardware.title')}</h1>
	<p class="mb-4 text-sm text-neutral-400">{t('hardware.hint')}</p>
	<button
		type="button"
		class="rounded-md bg-gold px-4 py-3 font-medium text-black hover:bg-[#d6bb66] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gold disabled:opacity-60"
		disabled={running}
		onclick={diagnose}
	>
		{running ? t('hardware.running') : t('hardware.run')}
	</button>
	{#if failed}<p role="alert" class="mt-3 text-sm text-red-400">{t('hardware.failed')}</p>{/if}

	<TranscodePool {report} {onsaved} />

	{#if report}
		<p class="mt-6 text-xs text-neutral-500">
			{t('hardware.generated', {
				date: new Date(report.generated_at).toLocaleString(),
				ms: report.duration_ms
			})}
		</p>

		{#if report.warnings.length}
			<ul class="mt-3 flex flex-col gap-2">
				{#each report.warnings as warning (warning)}
					<li
						class="rounded-lg bg-amber-500/10 px-3 py-2 text-sm text-amber-200 ring-1 ring-amber-400/30"
					>
						{t(`hardware.warnings.${warning}`)}
					</li>
				{/each}
			</ul>
		{/if}

		<div class="mt-4 grid gap-4 md:grid-cols-2">
			<div class={card}>
				<h2 class="text-sm text-neutral-400">{t('hardware.system')}</h2>
				<p class="mt-1 font-semibold">
					{report.os}
					{report.os_version ?? ''}
					<span class="font-mono text-sm text-neutral-400">{report.arch}</span>
				</p>
				{#if report.container}<span class="{chip} mt-2 inline-block">{t('hardware.container')}</span
					>{/if}
			</div>
			<div class={card}>
				<h2 class="text-sm text-neutral-400">{t('hardware.cpu')}</h2>
				<p class="mt-1 font-semibold">{report.cpu.model ?? t('hardware.unknown')}</p>
				<p class="mt-1 text-sm text-neutral-300">
					{t('hardware.cores', {
						physical: report.cpu.physical_cores ?? '–',
						logical: report.cpu.logical_cores
					})} · {t('hardware.ram', { value: mb(report.cpu.memory_total_mb) })}
				</p>
			</div>
		</div>

		<h2 class="mt-6 mb-3 font-semibold">{t('hardware.gpus', { count: report.gpus.length })}</h2>
		{#if report.gpus.length === 0}
			<p class="text-sm text-neutral-400">{t('hardware.no_gpus')}</p>
		{:else}
			<div class="grid gap-4 md:grid-cols-2">
				{#each report.gpus as gpu, i (i)}
					<div class={card}>
						<div class="flex flex-wrap items-center gap-2">
							<span
								class="rounded-full px-2 py-0.5 text-xs font-medium uppercase ring-1 {vendorTone[
									gpu.vendor
								] ?? 'bg-white/10 text-neutral-300 ring-white/20'}">{gpu.vendor}</span
							>
							<span class={chip}>{t(`hardware.kinds.${gpu.kind}`)}</span>
							{#if gpu.index !== null && gpu.index !== undefined}
								<span class={chip}>{t('hardware.gpu_n', { index: gpu.index })}</span>
							{/if}
							{#if gpu.render_node}<span class={chip}>{gpu.render_node}</span>{/if}
						</div>
						<p class="mt-2 font-semibold">{gpu.name}</p>
						{#if gpu.driver}<p class="text-xs text-neutral-400">
								{t('hardware.driver', { value: gpu.driver })}
							</p>{/if}
						{#if gpu.memory_total_mb}
							{@const used = gpu.memory_used_mb ?? 0}
							<p class="mt-3 flex justify-between text-xs text-neutral-400">
								<span>{t('hardware.vram')}</span>
								<span class="tabular-nums"
									>{gpu.memory_used_mb !== null && gpu.memory_used_mb !== undefined
										? `${mb(used)} / ${mb(gpu.memory_total_mb)}`
										: mb(gpu.memory_total_mb)}</span
								>
							</p>
							<span class="mt-1 block h-1.5 overflow-hidden rounded-full bg-white/10">
								<span
									class="block h-full rounded-full bg-gold"
									style:width={`${Math.min(100, (used / gpu.memory_total_mb) * 100)}%`}
								></span>
							</span>
						{/if}
						<p class="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-neutral-300">
							{#if gpu.utilization_percent !== null && gpu.utilization_percent !== undefined}
								<span>{t('hardware.utilization', { value: gpu.utilization_percent })}</span>
							{/if}
							{#if gpu.vendor === 'nvidia'}
								<span>{t('hardware.lisek_transcodes', { count: gpu.active_transcodes })}</span>
							{/if}
							{#if gpu.encoder_sessions !== null && gpu.encoder_sessions !== undefined}
								<span>{t('hardware.encoder_sessions', { count: gpu.encoder_sessions })}</span>
							{/if}
						</p>
					</div>
				{/each}
			</div>
		{/if}

		<h2 class="mt-6 mb-3 font-semibold">{t('hardware.encoders')}</h2>
		{#if report.encoders.length === 0}
			<p class="text-sm text-neutral-400">{t('hardware.no_encoders')}</p>
		{:else}
			<div class="overflow-x-auto rounded-lg ring-1 ring-white/10">
				<table class="w-full text-left text-sm">
					<thead class="bg-white/5 text-xs text-neutral-300">
						<tr>
							<th class="p-2.5">{t('hardware.col_encoder')}</th>
							<th class="p-2.5">{t('hardware.col_device')}</th>
							<th class="p-2.5">{t('hardware.col_result')}</th>
						</tr>
					</thead>
					<tbody>
						{#each report.encoders as test, i (i)}
							<tr class="border-t border-white/10 align-top">
								<td class="p-2.5 font-mono text-xs">
									{test.encoder}
									{#if test.used_by_lisek}<span
											class="mt-1 inline-block rounded bg-gold/15 px-1.5 py-0.5 font-sans text-[0.65rem] whitespace-nowrap text-gold"
											>{t('hardware.used_by_lisek')}</span
										>{/if}
								</td>
								<td class="p-2.5 text-xs text-neutral-300">{test.device ?? '–'}</td>
								<td class="p-2.5 text-xs">
									{#if test.ok}
										<span class="text-emerald-300">✓ {t('hardware.works')}</span>
									{:else}
										<span class="text-red-300">✗ {t('hardware.fails')}</span>
										{#if test.error}<span class="mt-0.5 block break-all text-neutral-500"
												>{test.error}</span
											>{/if}
									{/if}
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}

		{#if report.vaapi.length}
			<h2 class="mt-6 mb-3 font-semibold">{t('hardware.vaapi')}</h2>
			<div class="grid gap-4 md:grid-cols-2">
				{#each report.vaapi as device (device.render_node)}
					<div class={card}>
						<p class="font-mono text-xs text-neutral-300">{device.render_node}</p>
						{#if device.driver}<p class="mt-1 text-xs text-neutral-400">{device.driver}</p>{/if}
						<p class="mt-3 text-xs text-neutral-400">{t('hardware.decode')}</p>
						<p class="mt-1 flex flex-wrap gap-1">
							{#each device.decode as codec (codec)}<span class={chip}>{codec}</span>{/each}
						</p>
						<p class="mt-3 text-xs text-neutral-400">{t('hardware.encode')}</p>
						<p class="mt-1 flex flex-wrap gap-1">
							{#each device.encode as codec (codec)}<span class={chip}>{codec}</span>{/each}
						</p>
					</div>
				{/each}
			</div>
		{/if}

		<div class="{card} mt-6 text-sm">
			<h2 class="text-sm text-neutral-400">{t('hardware.ffmpeg')}</h2>
			<p class="mt-1 break-all">{report.ffmpeg.version ?? t('hardware.unknown')}</p>
			<p class="mt-1 text-xs break-all text-neutral-500">{report.ffmpeg.path}</p>
			<p class="mt-3 text-xs text-neutral-400">{t('hardware.hwaccels')}</p>
			<p class="mt-1 flex flex-wrap gap-1">
				{#each report.ffmpeg.hwaccels as method (method)}<span class={chip}>{method}</span
					>{:else}<span class="text-xs text-neutral-500">–</span>{/each}
			</p>
			{#if report.vulkan_devices.length}
				<p class="mt-3 text-xs text-neutral-400">{t('hardware.vulkan')}</p>
				<p class="mt-1 flex flex-wrap gap-1">
					{#each report.vulkan_devices as name (name)}<span class={chip}>{name}</span>{/each}
				</p>
			{/if}
		</div>
	{/if}
</section>
