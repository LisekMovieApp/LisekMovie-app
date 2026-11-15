<!--
  Faza 26 — own TV channels: the list (number, logo, what is on now,
  status), adding and editing a channel, logo upload, drag-and-drop order.
  The schedule of a channel has its own page.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		QUALITIES,
		browserTimezone,
		createChannel,
		deleteChannel,
		deleteLogo,
		errorCode,
		fetchChannels,
		fetchPresets,
		logoUrl,
		moveItem,
		reorderChannels,
		timezones,
		updateChannel,
		uploadLogo,
		type AdminChannel,
		type ChannelBody,
		type FillerPreset
	} from '$lib/tv/api';
	import Icon from '$lib/components/Icon.svelte';

	let token = $state('');
	let channels = $state<AdminChannel[]>([]);
	let presets = $state<FillerPreset[]>([]);
	let loading = $state(true);
	let loadFailed = $state(false);
	let errorKey = $state<string | null>(null);
	let editing = $state<AdminChannel | 'new' | null>(null);
	let form = $state<ChannelBody>(blank());
	let logoFile = $state<File | null>(null);
	let removeLogo = $state(false);
	let saving = $state(false);
	let confirmingId = $state<string | null>(null);
	let dragFrom = $state<number | null>(null);
	let logoVersion = $state(0);

	const zones = timezones();
	const fallbackPresets = $derived(presets.filter((p) => p.kind === 'fallback'));

	function blank(): ChannelBody {
		return {
			name: '',
			number: '',
			group_title: '',
			enabled: true,
			timezone: browserTimezone(),
			quality: '720p',
			fallback_filler_id: null,
			keep_running: false
		};
	}

	async function load() {
		try {
			channels = await fetchChannels(token);
			loadFailed = false;
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		void load();
		fetchPresets(token)
			.then((list) => (presets = list))
			.catch(() => {});
		const timer = setInterval(() => void load(), 15_000);
		return () => clearInterval(timer);
	});

	function startEdit(channel: AdminChannel | 'new') {
		editing = channel;
		errorKey = null;
		logoFile = null;
		removeLogo = false;
		form =
			channel === 'new'
				? blank()
				: {
						name: channel.name,
						number: channel.number,
						group_title: channel.group_title ?? '',
						enabled: channel.enabled,
						timezone: channel.timezone,
						quality: channel.quality,
						fallback_filler_id: channel.fallback_filler_id ?? null,
						keep_running: channel.keep_running
					};
	}

	const errorMessages = [
		'number_taken',
		'invalid_name',
		'invalid_number',
		'invalid_timezone',
		'invalid_quality',
		'invalid_group',
		'filler_not_found'
	];

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (!editing) return;
		saving = true;
		errorKey = null;
		try {
			const body: ChannelBody = {
				...form,
				name: form.name.trim(),
				number: (form.number ?? '').trim(),
				group_title: (form.group_title ?? '').trim() || null
			};
			const saved =
				editing === 'new'
					? await createChannel(token, body)
					: await updateChannel(token, editing.id, body);
			if (logoFile) await uploadLogo(token, saved.id, logoFile);
			else if (removeLogo && saved.has_logo) await deleteLogo(token, saved.id);
			logoVersion++;
			editing = null;
			await load();
		} catch (error) {
			const code = errorCode(error);
			errorKey = code && errorMessages.includes(code) ? `tv_admin.errors.${code}` : 'tv_admin.errors.save_failed';
		} finally {
			saving = false;
		}
	}

	async function remove(channel: AdminChannel) {
		if (confirmingId !== channel.id) {
			confirmingId = channel.id;
			return;
		}
		confirmingId = null;
		try {
			await deleteChannel(token, channel.id);
			channels = channels.filter((c) => c.id !== channel.id);
		} catch {
			errorKey = 'tv_admin.errors.delete_failed';
		}
	}

	async function move(from: number, to: number) {
		const next = moveItem(channels, from, to);
		if (next === channels) return;
		channels = next;
		try {
			await reorderChannels(
				token,
				next.map((c) => c.id)
			);
		} catch {
			errorKey = 'tv_admin.errors.reorder_failed';
			await load();
		}
	}

	function clock(iso: string | null | undefined) {
		return iso
			? new Intl.DateTimeFormat(getLocale(), { hour: '2-digit', minute: '2-digit' }).format(new Date(iso))
			: '';
	}

	const field =
		'rounded-md border border-neutral-700 bg-black px-3 py-2 text-sm text-neutral-100 focus:border-gold';
	const button =
		'rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:opacity-50';
</script>

<svelte:head><title>{t('tv_admin.title')} · LisekMovie</title></svelte:head>

<main class="min-h-screen bg-base px-4 py-8 md:px-8">
	<div class="mx-auto max-w-5xl">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div>
				<h1 class="text-2xl font-semibold text-neutral-100">{t('tv_admin.title')}</h1>
				<p class="mt-1 text-sm text-neutral-400">{t('tv_admin.subtitle')}</p>
			</div>
			<button
				type="button"
				class="flex items-center gap-1.5 rounded-md bg-gold px-4 py-2 text-sm font-medium text-black hover:bg-gold-dim"
				onclick={() => startEdit('new')}
			>
				<Icon name="plus" class="size-4" />
				{t('tv_admin.add_channel')}
			</button>
		</div>

		{#if errorKey}<p role="alert" class="mt-3 text-sm text-red-400">{t(errorKey)}</p>{/if}

		{#if editing}
			<form
				onsubmit={save}
				class="mt-6 grid gap-4 rounded-lg border border-white/10 bg-surface/60 p-4 sm:grid-cols-2"
			>
				<h2 class="text-lg font-semibold text-neutral-100 sm:col-span-2">
					{editing === 'new' ? t('tv_admin.new_channel') : t('tv_admin.edit_channel')}
				</h2>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('tv_admin.field_name')}
					<input class={field} bind:value={form.name} maxlength="100" required />
				</label>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('tv_admin.field_number')}
					<input
						class={field}
						bind:value={form.number}
						maxlength="8"
						placeholder={t('tv_admin.number_auto')}
						inputmode="decimal"
					/>
				</label>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('tv_admin.field_group')}
					<input class={field} bind:value={form.group_title} maxlength="100" />
				</label>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('tv_admin.field_timezone')}
					<select class={field} bind:value={form.timezone}>
						{#each zones as zone (zone)}<option value={zone}>{zone}</option>{/each}
					</select>
					<span class="text-xs text-neutral-500">{t('tv_admin.timezone_hint')}</span>
				</label>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('tv_admin.field_quality')}
					<select class={field} bind:value={form.quality}>
						{#each QUALITIES as quality (quality)}<option value={quality}>{quality}</option>{/each}
					</select>
				</label>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('tv_admin.field_fallback')}
					<select class={field} bind:value={form.fallback_filler_id}>
						<option value={null}>{t('tv_admin.fallback_none')}</option>
						{#each fallbackPresets as preset (preset.id)}
							<option value={preset.id}>{preset.name}</option>
						{/each}
					</select>
					<span class="text-xs text-neutral-500">{t('tv_admin.fallback_hint')}</span>
				</label>
				<label class="flex items-center gap-2 text-sm text-neutral-300">
					<input type="checkbox" bind:checked={form.enabled} />
					{t('tv_admin.field_enabled')}
				</label>
				<label class="flex items-center gap-2 text-sm text-neutral-300">
					<input type="checkbox" bind:checked={form.keep_running} />
					{t('tv_admin.field_keep_running')}
				</label>
				<div class="flex flex-col gap-2 text-sm text-neutral-300 sm:col-span-2">
					<span>{t('tv_admin.field_logo')}</span>
					<div class="flex flex-wrap items-center gap-3">
						{#if editing !== 'new' && editing.has_logo && !removeLogo && !logoFile}
							<img
								src={logoUrl(editing.id, token, logoVersion)}
								alt={t('tv_admin.logo_alt')}
								class="h-12 max-w-32 rounded bg-white/5 object-contain p-1"
							/>
							<button type="button" class={button} onclick={() => (removeLogo = true)}>
								{t('tv_admin.logo_remove')}
							</button>
						{/if}
						<input
							type="file"
							accept="image/png,image/jpeg,image/webp,image/gif"
							aria-label={t('tv_admin.logo_choose')}
							onchange={(event) => {
								logoFile = event.currentTarget.files?.[0] ?? null;
								if (logoFile) removeLogo = false;
							}}
						/>
					</div>
				</div>
				<div class="flex gap-2 sm:col-span-2">
					<button
						type="submit"
						disabled={saving || form.name.trim() === ''}
						class="rounded-md bg-gold px-4 py-2 text-sm font-medium text-black hover:bg-gold-dim disabled:opacity-50"
					>
						{saving ? t('tv_admin.saving') : t('tv_admin.save')}
					</button>
					<button type="button" class={button} onclick={() => (editing = null)}>
						{t('tv_admin.cancel')}
					</button>
				</div>
			</form>
		{/if}

		<section class="mt-8" aria-label={t('tv_admin.list_title')}>
			{#if loading}
				<p class="text-sm text-neutral-400">{t('admin.loading')}</p>
			{:else if loadFailed}
				<p class="text-sm text-red-400">{t('admin.load_failed')}</p>
			{:else if channels.length === 0}
				<p class="text-sm text-neutral-400">{t('tv_admin.none')}</p>
			{:else}
				<p class="mb-2 text-xs text-neutral-500">{t('tv_admin.drag_hint')}</p>
				<ul class="flex flex-col gap-2">
					{#each channels as channel, index (channel.id)}
						<li
							draggable="true"
							ondragstart={() => (dragFrom = index)}
							ondragover={(event) => event.preventDefault()}
							ondrop={() => {
								if (dragFrom !== null) void move(dragFrom, index);
								dragFrom = null;
							}}
							class="flex flex-wrap items-center gap-3 rounded-lg border border-white/10 bg-surface/60 p-3 {channel.enabled
								? ''
								: 'opacity-60'}"
						>
							<span class="cursor-grab text-neutral-500" aria-hidden="true">⋮⋮</span>
							<span class="w-10 text-center text-lg font-semibold text-gold">{channel.number}</span>
							{#if channel.has_logo}
								<img
									src={logoUrl(channel.id, token, logoVersion)}
									alt=""
									class="h-9 w-16 rounded bg-white/5 object-contain p-0.5"
								/>
							{:else}
								<span class="flex h-9 w-16 items-center justify-center rounded bg-white/5 text-neutral-600">
									<Icon name="tv" class="size-5" />
								</span>
							{/if}
							<div class="min-w-0 flex-1">
								<p class="truncate font-medium text-neutral-100">{channel.name}</p>
								<p class="truncate text-xs text-neutral-400">
									{channel.group_title ?? t('tv_admin.no_group')} · {channel.quality} · {channel.timezone}
									· {t('tv_admin.slot_count', { count: channel.slot_count })}
								</p>
								<p class="truncate text-xs text-neutral-300">
									{#if channel.now_title}
										{t('tv_admin.now_playing', {
											title: channel.now_title,
											until: clock(channel.now_ends_at)
										})}
									{:else}
										{t('tv_admin.nothing_planned')}
									{/if}
								</p>
								{#if channel.last_error}
									<p class="truncate text-xs text-amber-400">{channel.last_error}</p>
								{/if}
							</div>
							<div class="flex flex-wrap items-center gap-1.5 text-xs">
								{#if !channel.enabled}
									<span class="rounded bg-white/10 px-2 py-0.5 text-neutral-300">{t('tv_admin.status_off')}</span>
								{/if}
								{#if channel.streaming}
									<span class="rounded bg-green-500/20 px-2 py-0.5 text-green-300">
										{t('tv_admin.status_streaming', { count: channel.viewers })}
									</span>
								{/if}
								{#if channel.dirty}
									<span class="rounded bg-amber-500/20 px-2 py-0.5 text-amber-300">{t('tv_admin.status_planning')}</span>
								{/if}
							</div>
							<div class="flex items-center gap-1.5">
								<button
									type="button"
									class={button}
									disabled={index === 0}
									aria-label={t('tv_admin.move_up')}
									onclick={() => move(index, index - 1)}>↑</button
								>
								<button
									type="button"
									class={button}
									disabled={index === channels.length - 1}
									aria-label={t('tv_admin.move_down')}
									onclick={() => move(index, index + 1)}>↓</button
								>
								<a href={resolve('/admin/tv/channel/[id]', { id: channel.id })} class={button}>
									{t('tv_admin.schedule')}
								</a>
								<button type="button" class={button} onclick={() => startEdit(channel)}>
									{t('tv_admin.edit')}
								</button>
								<button
									type="button"
									class="{button} {confirmingId === channel.id ? 'border-red-500 text-red-300' : ''}"
									onclick={() => remove(channel)}
								>
									{confirmingId === channel.id ? t('tv_admin.confirm_delete') : t('tv_admin.delete')}
								</button>
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
	</div>
</main>
