<!--
  Faza 26 — IPTV channels (Faza 20 sources): turn single channels on and off,
  override their number, group and logo, see channels that keep failing, and
  copy the links to give another IPTV player (Kodi, VLC, TiviMate …).
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchLiveSources, refreshLiveSource, type LiveTvSource } from '$lib/livetv/api';
	import {
		blankToNull,
		fetchLiveChannels,
		iptvLinks,
		saveLiveChannel,
		type AdminLiveChannel
	} from '$lib/tv/api';

	type Draft = { enabled: boolean; number: string; group: string; logo: string };

	let token = '';
	let sources = $state<LiveTvSource[]>([]);
	let selected = $state<string | null>(null);
	let channels = $state<AdminLiveChannel[]>([]);
	let drafts = $state<Record<string, Draft>>({});
	let filter = $state('');
	let loading = $state(true);
	let errorKey = $state<string | null>(null);
	let savedId = $state<string | null>(null);
	let refreshing = $state(false);

	const origin = typeof location === 'undefined' ? '' : location.origin;
	const links = iptvLinks(origin, null);
	const shown = $derived(
		channels.filter((c) => c.name.toLowerCase().includes(filter.trim().toLowerCase()))
	);

	function draftOf(channel: AdminLiveChannel): Draft {
		return {
			enabled: channel.enabled,
			number: channel.number_override ?? '',
			group: channel.group_override ?? '',
			logo: channel.logo_override ?? ''
		};
	}

	async function open(libraryId: string) {
		selected = libraryId;
		errorKey = null;
		try {
			channels = await fetchLiveChannels(token, libraryId);
			drafts = Object.fromEntries(channels.map((c) => [c.id, draftOf(c)]));
		} catch {
			errorKey = 'admin.load_failed';
		}
	}

	onMount(async () => {
		token = getSessionCookieClientSide() ?? '';
		try {
			sources = await fetchLiveSources(token);
			if (sources.length > 0) await open(sources[0].library_id);
		} catch {
			errorKey = 'admin.load_failed';
		} finally {
			loading = false;
		}
	});

	const changed = (channel: AdminLiveChannel) =>
		JSON.stringify(drafts[channel.id]) !== JSON.stringify(draftOf(channel));

	async function save(channel: AdminLiveChannel) {
		const draft = drafts[channel.id];
		errorKey = null;
		try {
			await saveLiveChannel(token, channel.id, {
				enabled: draft.enabled,
				number_override: blankToNull(draft.number),
				group_override: blankToNull(draft.group),
				logo_override: blankToNull(draft.logo)
			});
			channels = channels.map((c) =>
				c.id === channel.id
					? {
							...c,
							enabled: draft.enabled,
							number_override: blankToNull(draft.number),
							group_override: blankToNull(draft.group),
							logo_override: blankToNull(draft.logo),
							failure_count: draft.enabled && !c.enabled ? 0 : c.failure_count
						}
					: c
			);
			savedId = channel.id;
		} catch {
			errorKey = 'tv_admin.errors.channel_settings_failed';
		}
	}

	async function refresh() {
		if (!selected) return;
		refreshing = true;
		try {
			const updated = await refreshLiveSource(token, selected);
			sources = sources.map((s) => (s.library_id === updated.library_id ? updated : s));
			await open(selected);
		} catch {
			errorKey = 'tv_admin.errors.refresh_failed';
		} finally {
			refreshing = false;
		}
	}

	const when = (iso: string | null) =>
		iso ? new Date(iso).toLocaleString(getLocale(), { dateStyle: 'short', timeStyle: 'short' }) : t('tv_admin.never_scanned');
	const field = 'rounded-md border border-neutral-700 bg-black px-2 py-1.5 text-sm text-neutral-100 focus:border-gold';
	const button = 'rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50';
</script>

<svelte:head><title>{t('tv_admin.tab_iptv')} · LisekMovie</title></svelte:head>

<main class="min-h-screen bg-base px-4 py-8 md:px-8">
	<div class="mx-auto max-w-5xl">
		<h1 class="text-2xl font-semibold text-neutral-100">{t('tv_admin.tab_iptv')}</h1>
		<p class="mt-1 text-sm text-neutral-400">{t('tv_admin.iptv_hint')}</p>
		{#if errorKey}<p role="alert" class="mt-3 text-sm text-red-400">{t(errorKey)}</p>{/if}

		<section class="mt-6 rounded-lg border border-white/10 bg-surface/60 p-4" aria-label={t('tv_admin.export_title')}>
			<h2 class="text-lg font-semibold text-neutral-100">{t('tv_admin.export_title')}</h2>
			<p class="mt-1 text-sm text-neutral-400">
				{t('tv_admin.export_hint')}
				<a href={resolve('/admin/api')} class="text-gold hover:underline">{t('tv_admin.export_keys_link')}</a>
			</p>
			<dl class="mt-3 grid gap-2 text-sm">
				<div>
					<dt class="text-xs text-neutral-500">{t('tv_admin.export_playlist')}</dt>
					<dd><code class="break-all rounded bg-black/40 px-2 py-1 text-xs">{`${links.playlist}?key=<${t('api_keys.key_placeholder')}>`}</code></dd>
				</div>
				<div>
					<dt class="text-xs text-neutral-500">{t('tv_admin.export_guide')}</dt>
					<dd><code class="break-all rounded bg-black/40 px-2 py-1 text-xs">{`${links.guide}?key=<${t('api_keys.key_placeholder')}>`}</code></dd>
				</div>
			</dl>
			<p class="mt-3 text-xs text-amber-400">{t('tv_admin.export_warning')}</p>
		</section>

		{#if loading}
			<p class="mt-6 text-sm text-neutral-400">{t('admin.loading')}</p>
		{:else if sources.length === 0}
			<p class="mt-6 text-sm text-neutral-400">
				{t('tv_admin.no_sources')}
				<a href={resolve('/admin/libraries')} class="text-gold hover:underline">{t('tv_admin.sources_link')}</a>
			</p>
		{:else}
			<div class="mt-6 flex flex-wrap items-center gap-2">
				{#each sources as source (source.library_id)}
					<button type="button" aria-pressed={selected === source.library_id}
						class="rounded-full px-4 py-1.5 text-sm {selected === source.library_id ? 'bg-gold text-black' : 'bg-white/5 text-neutral-300'}"
						onclick={() => open(source.library_id)}>
						{source.library_name}
					</button>
				{/each}
			</div>
			{#each sources.filter((s) => s.library_id === selected) as source (source.library_id)}
				<p class="mt-3 text-xs text-neutral-400">
					{t('tv_admin.source_state', { count: source.channel_count, when: when(source.refreshed_at) })}
					{#if source.last_error}<span class="text-amber-400"> · {source.last_error}</span>{/if}
				</p>
			{/each}
			<div class="mt-3 flex items-center gap-2">
				<input class="{field} w-64" type="search" bind:value={filter} placeholder={t('tv_admin.filter_channels')} aria-label={t('tv_admin.filter_channels')} />
				<button type="button" class={button} disabled={refreshing} onclick={refresh}>
					{refreshing ? t('tv_admin.refreshing') : t('tv_admin.refresh_source')}
				</button>
			</div>

			<div class="mt-3 overflow-x-auto rounded-lg ring-1 ring-white/10">
				<table class="w-full text-left text-sm">
					<thead class="bg-white/5 text-neutral-300">
						<tr>
							<th class="p-2 font-medium">{t('tv_admin.col_on')}</th>
							<th class="p-2 font-medium">{t('tv_admin.col_channel')}</th>
							<th class="p-2 font-medium">{t('tv_admin.col_number')}</th>
							<th class="p-2 font-medium">{t('tv_admin.col_group')}</th>
							<th class="p-2 font-medium">{t('tv_admin.col_logo')}</th>
							<th class="p-2 font-medium">{t('tv_admin.col_failures')}</th>
							<th class="p-2"><span class="sr-only">{t('api_keys.col_actions')}</span></th>
						</tr>
					</thead>
					<tbody>
						{#each shown as channel (channel.id)}
							{#if drafts[channel.id]}
								<tr class="border-t border-white/5 align-middle {drafts[channel.id].enabled ? '' : 'opacity-60'}">
									<td class="p-2"><input type="checkbox" bind:checked={drafts[channel.id].enabled} aria-label={t('tv_admin.col_on')} /></td>
									<td class="p-2 text-neutral-100">{channel.name}</td>
									<td class="p-2"><input class="{field} w-20" bind:value={drafts[channel.id].number} placeholder={channel.number ?? ''} maxlength="20" aria-label={t('tv_admin.col_number')} /></td>
									<td class="p-2"><input class="{field} w-36" bind:value={drafts[channel.id].group} placeholder={channel.group_title ?? ''} maxlength="100" aria-label={t('tv_admin.col_group')} /></td>
									<td class="p-2"><input class="{field} w-44" bind:value={drafts[channel.id].logo} placeholder={channel.logo_url ?? ''} maxlength="500" aria-label={t('tv_admin.col_logo')} /></td>
									<td class="p-2 whitespace-nowrap {channel.failure_count >= 3 ? 'text-red-400' : 'text-neutral-400'}">
										{channel.failure_count}
										{#if channel.last_failure_at}<span class="text-xs text-neutral-500">· {when(channel.last_failure_at)}</span>{/if}
									</td>
									<td class="p-2 text-right">
										<button type="button" class={button} disabled={!changed(channel)} onclick={() => save(channel)}>
											{savedId === channel.id && !changed(channel) ? t('tv_admin.saved') : t('tv_admin.save')}
										</button>
									</td>
								</tr>
							{/if}
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
	</div>
</main>
