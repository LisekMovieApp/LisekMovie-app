<!--
  Faza 26 — ads and bumpers: folders of video files on the server (scanned
  for durations), and presets that say how they are used — before or after a
  programme, to fill the tail of a slot, when a channel has nothing to play.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		FILLER_KINDS,
		FILLER_MODES,
		addFolder,
		blankPreset,
		deleteFolder,
		deletePreset,
		errorCode,
		fetchFolderFiles,
		fetchFolders,
		fetchPresets,
		formatLength,
		normalizePreset,
		savePreset,
		scanFolder,
		setFolderEnabled,
		type FillerFile,
		type FillerFolder,
		type FillerPreset
	} from '$lib/tv/api';

	let token = '';
	let folders = $state<FillerFolder[]>([]);
	let presets = $state<FillerPreset[]>([]);
	let loading = $state(true);
	let errorKey = $state<string | null>(null);
	let path = $state('');
	let opened = $state<{ id: string; files: FillerFile[] } | null>(null);
	let confirmingId = $state<string | null>(null);
	let form = $state<FillerPreset | null>(null);

	async function load() {
		try {
			[folders, presets] = await Promise.all([fetchFolders(token), fetchPresets(token)]);
		} catch {
			errorKey = 'admin.load_failed';
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		void load();
		// A new folder is scanned in the background.
		const timer = setInterval(() => void load(), 5000);
		return () => clearInterval(timer);
	});

	async function add(event: SubmitEvent) {
		event.preventDefault();
		errorKey = null;
		try {
			await addFolder(token, path.trim());
			path = '';
			await load();
		} catch (error) {
			const code = errorCode(error);
			errorKey = code === 'folder_exists' ? 'tv_admin.errors.folder_exists' : code === 'invalid_path' ? 'tv_admin.errors.invalid_path' : 'tv_admin.errors.save_failed';
		}
	}

	async function toggleFiles(folder: FillerFolder) {
		if (opened?.id === folder.id) {
			opened = null;
			return;
		}
		opened = { id: folder.id, files: await fetchFolderFiles(token, folder.id).catch(() => []) };
	}

	async function removeFolder(folder: FillerFolder) {
		if (confirmingId !== folder.id) {
			confirmingId = folder.id;
			return;
		}
		confirmingId = null;
		await deleteFolder(token, folder.id).catch(() => (errorKey = 'tv_admin.errors.delete_failed'));
		await load();
	}

	async function toggleFolder(folder: FillerFolder) {
		await setFolderEnabled(token, folder.id, !folder.enabled).catch(() => (errorKey = 'tv_admin.errors.save_failed'));
		await load();
	}

	function folderNote(folder: FillerFolder): string | null {
		const error = folder.last_error;
		if (!error) return null;
		if (error === 'folder_missing') return t('tv_admin.folder_missing');
		if (error.startsWith('unreadable_files:')) return t('tv_admin.folder_unreadable', { count: error.split(':')[1] });
		return error;
	}

	async function submitPreset(event: SubmitEvent) {
		event.preventDefault();
		if (!form) return;
		errorKey = null;
		try {
			await savePreset(token, normalizePreset({ ...form, name: form.name.trim() }));
			form = null;
			await load();
		} catch (error) {
			errorKey = errorCode(error) === 'name_taken' ? 'tv_admin.errors.name_taken' : 'tv_admin.errors.save_failed';
		}
	}

	async function removePreset(preset: FillerPreset) {
		if (!preset.id) return;
		if (confirmingId !== preset.id) {
			confirmingId = preset.id;
			return;
		}
		confirmingId = null;
		await deletePreset(token, preset.id).catch(() => (errorKey = 'tv_admin.errors.delete_failed'));
		await load();
	}

	function toggleFolderIn(id: string) {
		if (!form) return;
		const ids = form.folder_ids ?? [];
		form = { ...form, folder_ids: ids.includes(id) ? ids.filter((i) => i !== id) : [...ids, id] };
	}

	const when = (iso: string | null | undefined) =>
		iso ? new Date(iso).toLocaleString(getLocale(), { dateStyle: 'short', timeStyle: 'short' }) : t('tv_admin.never_scanned');
	const field = 'rounded-md border border-neutral-700 bg-black px-3 py-2 text-sm text-neutral-100 focus:border-gold';
	const button = 'rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50';
</script>

<svelte:head><title>{t('tv_admin.tab_fillers')} · LisekMovie</title></svelte:head>

<main class="min-h-screen bg-base px-4 py-8 md:px-8">
	<div class="mx-auto max-w-5xl">
		<h1 class="text-2xl font-semibold text-neutral-100">{t('tv_admin.tab_fillers')}</h1>
		<p class="mt-1 text-sm text-neutral-400">{t('tv_admin.fillers_hint')}</p>
		{#if errorKey}<p role="alert" class="mt-3 text-sm text-red-400">{t(errorKey)}</p>{/if}

		<section class="mt-6" aria-label={t('tv_admin.folders')}>
			<h2 class="text-lg font-semibold text-neutral-100">{t('tv_admin.folders')}</h2>
			<form onsubmit={add} class="mt-2 flex flex-col gap-2 sm:flex-row">
				<input class="{field} flex-1" bind:value={path} placeholder={t('tv_admin.folder_placeholder')} aria-label={t('tv_admin.folder_path')} />
				<button type="submit" disabled={path.trim() === ''} class="rounded-md bg-gold px-4 py-2 text-sm font-medium text-black hover:bg-gold-dim disabled:opacity-50">
					{t('tv_admin.add_folder')}
				</button>
			</form>
			{#if loading}
				<p class="mt-3 text-sm text-neutral-400">{t('admin.loading')}</p>
			{:else if folders.length === 0}
				<p class="mt-3 text-sm text-neutral-400">{t('tv_admin.no_folders')}</p>
			{:else}
				<ul class="mt-3 flex flex-col gap-2">
					{#each folders as folder (folder.id)}
						<li class="rounded-lg border border-white/10 bg-surface/60 p-3 {folder.enabled ? '' : 'opacity-60'}">
							<div class="flex flex-wrap items-center gap-3">
								<div class="min-w-0 flex-1">
									<p class="truncate font-mono text-sm text-neutral-100">{folder.path}</p>
									<p class="text-xs text-neutral-400">
										{t('tv_admin.file_count', { count: folder.file_count })} · {when(folder.scanned_at)}
									</p>
									{#if folderNote(folder)}<p class="text-xs text-amber-400">{folderNote(folder)}</p>{/if}
								</div>
								<button type="button" class={button} onclick={() => toggleFiles(folder)}>{t('tv_admin.files')}</button>
								<button type="button" class={button} onclick={() => scanFolder(token, folder.id).then(load)}>{t('tv_admin.rescan')}</button>
								<button type="button" class={button} onclick={() => toggleFolder(folder)}>
									{folder.enabled ? t('tv_admin.disable') : t('tv_admin.enable')}
								</button>
								<button type="button" class="{button} {confirmingId === folder.id ? 'border-red-500 text-red-300' : ''}" onclick={() => removeFolder(folder)}>
									{confirmingId === folder.id ? t('tv_admin.confirm_delete') : t('tv_admin.delete')}
								</button>
							</div>
							{#if opened?.id === folder.id}
								<ul class="mt-3 max-h-56 overflow-y-auto rounded bg-black/30 p-2 text-xs text-neutral-300">
									{#each opened.files as file (file.id)}
										<li class="flex justify-between gap-3 py-0.5">
											<span class="truncate font-mono">{file.path}</span>
											<span class="whitespace-nowrap text-neutral-500">{formatLength(file.duration_ms)}</span>
										</li>
									{:else}
										<li>{t('tv_admin.no_files')}</li>
									{/each}
								</ul>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section class="mt-10" aria-label={t('tv_admin.presets')}>
			<div class="flex items-center justify-between gap-3">
				<h2 class="text-lg font-semibold text-neutral-100">{t('tv_admin.presets')}</h2>
				<button type="button" class={button} onclick={() => (form = blankPreset())}>{t('tv_admin.add_preset')}</button>
			</div>
			<p class="text-xs text-neutral-500">{t('tv_admin.presets_hint')}</p>

			{#if form}
				<form onsubmit={submitPreset} class="mt-3 grid gap-3 rounded-lg border border-white/10 bg-surface/60 p-4 sm:grid-cols-2">
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('tv_admin.preset_name')}
						<input class={field} bind:value={form.name} maxlength="100" required />
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('tv_admin.preset_kind')}
						<select class={field} bind:value={form.kind}>
							{#each FILLER_KINDS as kind (kind)}<option value={kind}>{t(`tv_admin.preset_kinds.${kind}`)}</option>{/each}
						</select>
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('tv_admin.preset_mode')}
						<select class={field} bind:value={form.mode} onchange={() => form && (form = normalizePreset(form))}>
							{#each FILLER_MODES as mode (mode)}<option value={mode}>{t(`tv_admin.preset_modes.${mode}`)}</option>{/each}
						</select>
					</label>
					{#if form.mode === 'count'}
						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{t('tv_admin.preset_count')}
							<input type="number" min="1" max="50" class={field} bind:value={form.count} />
						</label>
					{:else if form.mode === 'duration'}
						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{t('tv_admin.preset_seconds')}
							<input type="number" min="1" max="86400" class={field} bind:value={form.duration_seconds} />
						</label>
					{:else}
						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{t('tv_admin.preset_pad')}
							<input type="number" min="1" max="240" class={field} bind:value={form.pad_minutes} />
						</label>
					{/if}
					<fieldset class="sm:col-span-2">
						<legend class="text-sm text-neutral-300">{t('tv_admin.preset_folders')}</legend>
						<div class="mt-1 flex flex-wrap gap-3">
							{#each folders as folder (folder.id)}
								<label class="flex items-center gap-2 text-sm text-neutral-300">
									<input type="checkbox" checked={(form.folder_ids ?? []).includes(folder.id)} onchange={() => toggleFolderIn(folder.id)} />
									<span class="font-mono text-xs">{folder.path}</span>
								</label>
							{:else}
								<span class="text-xs text-neutral-500">{t('tv_admin.no_folders')}</span>
							{/each}
						</div>
					</fieldset>
					<div class="flex gap-2 sm:col-span-2">
						<button type="submit" class="rounded-md bg-gold px-4 py-2 text-sm font-medium text-black hover:bg-gold-dim">{t('tv_admin.save')}</button>
						<button type="button" class={button} onclick={() => (form = null)}>{t('tv_admin.cancel')}</button>
					</div>
				</form>
			{/if}

			<ul class="mt-3 flex flex-col gap-2">
				{#each presets as preset (preset.id)}
					<li class="flex flex-wrap items-center gap-3 rounded-lg border border-white/10 bg-surface/60 p-3">
						<div class="min-w-0 flex-1">
							<p class="font-medium text-neutral-100">{preset.name}</p>
							<p class="text-xs text-neutral-400">
								{t(`tv_admin.preset_kinds.${preset.kind}`)} · {t(`tv_admin.preset_modes.${preset.mode}`)}
								{#if preset.mode === 'count'}: {preset.count}{:else if preset.mode === 'duration'}: {`${preset.duration_seconds} s`}{:else}: {`${preset.pad_minutes} min`}{/if}
								· {t('tv_admin.folder_count', { count: (preset.folder_ids ?? []).length })}
							</p>
						</div>
						<button type="button" class={button} onclick={() => (form = { ...preset })}>{t('tv_admin.edit')}</button>
						<button type="button" class="{button} {confirmingId === preset.id ? 'border-red-500 text-red-300' : ''}" onclick={() => removePreset(preset)}>
							{confirmingId === preset.id ? t('tv_admin.confirm_delete') : t('tv_admin.delete')}
						</button>
					</li>
				{:else}
					<li class="text-sm text-neutral-400">{t('tv_admin.no_presets')}</li>
				{/each}
			</ul>
		</section>
	</div>
</main>
