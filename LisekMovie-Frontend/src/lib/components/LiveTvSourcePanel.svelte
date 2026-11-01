<!--
  Faza 20 — a Live TV library in Admin → Libraries: its channel list and
  guide links, what the last refresh brought (or why it failed), "Refresh
  now", and editing the links.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchLiveSources,
		refreshLiveSource,
		setLiveSource,
		type LiveTvSource
	} from '$lib/livetv/api';
	import { relativeTime } from '$lib/users/profile';

	interface Props {
		libraryId: string;
	}

	let { libraryId }: Props = $props();

	let source = $state<LiveTvSource | null>(null);
	let busy = $state(false);
	let editing = $state(false);
	let failed = $state(false);
	let m3u = $state('');
	let xmltv = $state('');

	function token() {
		return getSessionCookieClientSide() ?? '';
	}

	async function load() {
		try {
			source = (await fetchLiveSources(token())).find((s) => s.library_id === libraryId) ?? null;
		} catch {
			failed = true;
		}
	}

	async function refresh() {
		busy = true;
		failed = false;
		try {
			source = await refreshLiveSource(token(), libraryId);
		} catch {
			failed = true;
		} finally {
			busy = false;
		}
	}

	function edit() {
		m3u = source?.m3u_url ?? '';
		xmltv = source?.xmltv_url ?? '';
		editing = true;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		busy = true;
		failed = false;
		try {
			source = await setLiveSource(token(), libraryId, m3u.trim(), xmltv.trim() || null);
			editing = false;
		} catch {
			failed = true;
		} finally {
			busy = false;
		}
	}

	onMount(() => {
		void load();
		// The first refresh after creating runs in the background.
		const timer = setInterval(() => {
			if (!source?.refreshed_at) void load();
		}, 5000);
		return () => clearInterval(timer);
	});
</script>

<div class="mt-3 rounded-md border border-white/10 bg-black/30 p-3 text-sm">
	{#if !source}
		<p class="text-neutral-500">{failed ? t('admin.livetv.load_failed') : t('common.loading')}</p>
	{:else}
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div class="min-w-0">
				{#if !source.refreshed_at}
					<p class="text-neutral-400" role="status">{t('admin.livetv.first_refresh')}</p>
				{:else}
					<p class="text-neutral-200">
						{t('admin.livetv.counts', {
							channels: String(source.channel_count),
							programs: String(source.program_count)
						})}
						<span class="text-neutral-500">
							· {t('admin.livetv.refreshed', {
								time: relativeTime(source.refreshed_at, getLocale())
							})}</span
						>
					</p>
				{/if}
				{#if source.last_error}
					<p class="mt-1 text-red-300" role="alert">
						{t('admin.livetv.last_error', { error: source.last_error })}
					</p>
				{/if}
				<p class="mt-1 truncate text-xs text-neutral-500" title={source.m3u_url}>
					{t('admin.livetv.m3u_line', { url: source.m3u_url })}
				</p>
				{#if source.xmltv_url}<p class="truncate text-xs text-neutral-500" title={source.xmltv_url}>
						{t('admin.livetv.xmltv_line', { url: source.xmltv_url })}
					</p>{/if}
			</div>
			<div class="flex gap-2">
				<button
					type="button"
					disabled={busy}
					onclick={edit}
					class="rounded-md border border-neutral-700 px-3 py-1.5 text-neutral-200 hover:border-gold-dim disabled:opacity-50"
					>{t('admin.livetv.edit')}</button
				>
				<button
					type="button"
					disabled={busy}
					onclick={refresh}
					class="rounded-md border border-neutral-700 px-3 py-1.5 text-neutral-200 hover:border-gold-dim disabled:opacity-50"
					>{busy ? t('admin.livetv.refreshing') : t('admin.livetv.refresh')}</button
				>
			</div>
		</div>
		{#if editing}
			<form onsubmit={save} class="mt-3 flex flex-col gap-2 border-t border-white/10 pt-3">
				<label class="flex flex-col gap-1 text-neutral-300">
					{t('admin.livetv.m3u_label')}
					<input
						bind:value={m3u}
						required
						class="rounded-md border border-neutral-700 bg-black px-3 py-2"
					/>
				</label>
				<label class="flex flex-col gap-1 text-neutral-300">
					{t('admin.livetv.xmltv_label')}
					<input
						bind:value={xmltv}
						class="rounded-md border border-neutral-700 bg-black px-3 py-2"
					/>
				</label>
				<div class="flex justify-end gap-2">
					<button
						type="button"
						class="rounded-md border border-neutral-700 px-3 py-1.5"
						onclick={() => (editing = false)}>{t('admin.cancel')}</button
					>
					<button
						disabled={busy}
						class="rounded-md bg-gold px-3 py-1.5 font-medium text-black disabled:opacity-50"
						>{t('admin.save')}</button
					>
				</div>
			</form>
		{/if}
		{#if failed && source}<p class="mt-2 text-red-300" role="alert">
				{t('admin.livetv.action_failed')}
			</p>{/if}
	{/if}
</div>
