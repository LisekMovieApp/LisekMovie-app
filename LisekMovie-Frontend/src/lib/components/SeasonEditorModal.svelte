<!--
  Faza 21: the metadata editor of one season (Serial → Sezon → Odcinek):
  its name, year, synopsis and poster. As in the item editor, "active
  fields" are the inverse of the season's locks; a field changed here is
  locked automatically on save, so a provider refresh keeps it. A replaced
  poster is saved at once and locked by the server.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		editorErrorKey,
		fetchSeason,
		updateSeason,
		type SeasonDetail
	} from '$lib/admin/mediaItems';
	import { showToast } from '$lib/toast/toast.svelte';
	import ArtworkEditor from './ArtworkEditor.svelte';

	const FIELDS = ['name', 'air_year', 'overview', 'poster'] as const;

	interface Props {
		/** Any episode of the series. */
		itemId: string;
		season: number;
		onClose: () => void;
		/** After a save or a replaced poster. */
		onSaved: () => void;
	}
	let { itemId, season, onClose, onSaved }: Props = $props();

	let loading = $state(true);
	let loadFailed = $state(false);
	let detail = $state<SeasonDetail | null>(null);
	let name = $state('');
	let airYear = $state('');
	let overview = $state('');
	let locked = $state<string[]>([]);
	let posterReplaced = $state(false);
	let saving = $state(false);
	let errorKey = $state<string | null>(null);

	let initial = $state<Record<string, string>>({});
	/** A number input's bound value is a number (or null) once edited. */
	const text = (value: unknown) =>
		(value === null || value === undefined ? '' : String(value)).trim();
	const snapshot = () => ({
		name: name.trim(),
		air_year: text(airYear),
		overview: overview.trim()
	});
	const current = $derived<Record<string, string>>(snapshot());
	const changed = $derived<string[]>(
		FIELDS.filter((field) => field in current && current[field] !== initial[field])
	);
	const isLocked = (field: string) =>
		locked.includes(field) || changed.includes(field) || (field === 'poster' && posterReplaced);

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;
		loading = true;
		loadFailed = false;
		try {
			const loaded = await fetchSeason(token, itemId, season);
			detail = loaded;
			name = loaded.name ?? '';
			airYear = loaded.air_year !== null ? String(loaded.air_year) : '';
			overview = loaded.overview ?? '';
			locked = [...loaded.locked_fields];
			initial = snapshot();
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	load();

	function toggleActive(field: string) {
		if (isLocked(field)) {
			locked = locked.filter((f) => f !== field);
			if (field === 'poster') posterReplaced = false;
		} else {
			locked = [...locked, field];
		}
	}

	async function handleSave(event: SubmitEvent) {
		event.preventDefault();
		const token = getSessionCookieClientSide();
		if (!token || !detail) return;
		const year = text(airYear);
		if (year !== '' && !/^\d{4}$/.test(year)) {
			errorKey = 'admin.editor_errors.invalid_year';
			return;
		}
		saving = true;
		errorKey = null;
		try {
			await updateSeason(token, itemId, season, {
				name: name.trim() || null,
				overview: overview.trim() || null,
				air_year: year === '' ? null : Number(year),
				locked_fields: FIELDS.filter((field) => isLocked(field))
			});
			showToast('success', t('admin.season_editor.saved'));
			onSaved();
			onClose();
		} catch (error) {
			errorKey = editorErrorKey(error);
		} finally {
			saving = false;
		}
	}

	function close() {
		if (posterReplaced) onSaved();
		onClose();
	}

	const input =
		'rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold';
</script>

<svelte:window onkeydown={(event) => event.key === 'Escape' && close()} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
	onclick={(event) => event.target === event.currentTarget && close()}
>
	<div
		role="dialog"
		aria-modal="true"
		aria-labelledby="season-editor-title"
		class="flex max-h-[92vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-white/10 bg-surface"
	>
		<header class="border-b border-white/10 px-6 py-4">
			<h2 id="season-editor-title" class="text-lg font-semibold text-neutral-100">
				{t('admin.season_editor.title')}
			</h2>
			{#if detail}
				<p class="mt-0.5 truncate text-sm text-neutral-400">
					{t('admin.season_editor.subtitle', {
						series: detail.series_title,
						season: String(detail.season),
						count: String(detail.episode_count)
					})}
				</p>
			{/if}
		</header>

		<div class="overflow-y-auto px-6 py-5">
			{#if loading}
				<p class="text-sm text-neutral-400">{t('admin.editor_loading')}</p>
			{:else if loadFailed || !detail}
				<p class="text-sm text-red-400">{t('admin.editor_load_failed')}</p>
			{:else}
				<form onsubmit={handleSave} class="grid gap-4 sm:grid-cols-[11rem_1fr]">
					<ArtworkEditor
						{itemId}
						kind="poster"
						{season}
						version={detail.version}
						onchanged={() => (posterReplaced = true)}
					/>
					<div class="flex flex-col gap-3">
						<div class="grid gap-3 sm:grid-cols-[1fr_7rem]">
							<label class="flex flex-col gap-1 text-sm text-neutral-300">
								{t('admin.season_editor.field_name')}
								<input
									type="text"
									maxlength="200"
									bind:value={name}
									placeholder={t('admin.season_editor.name_placeholder', {
										season: String(detail.season)
									})}
									class={input}
								/>
							</label>
							<label class="flex flex-col gap-1 text-sm text-neutral-300">
								{t('admin.season_editor.field_air_year')}
								<input type="number" min="1800" max="2200" bind:value={airYear} class={input} />
							</label>
						</div>
						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{t('admin.season_editor.field_overview')}
							<textarea bind:value={overview} rows="6" class="resize-y {input}"></textarea>
						</label>
					</div>

					<div class="rounded-md border border-gold/30 bg-gold/5 p-3 sm:col-span-2">
						<h3 class="text-sm font-semibold text-gold">{t('admin.editor_active_fields_title')}</h3>
						<p class="mt-1 text-xs text-neutral-400">{t('admin.editor_active_fields_hint')}</p>
						<div class="mt-3 grid gap-2 sm:grid-cols-2">
							{#each FIELDS as field (field)}
								<label class="flex items-center gap-2 text-sm text-neutral-200">
									<input
										type="checkbox"
										checked={!isLocked(field)}
										disabled={changed.includes(field)}
										onchange={() => toggleActive(field)}
										class="size-4 rounded border-neutral-600 bg-black accent-gold"
									/>
									{t(`admin.season_editor.field_${field}`)}
									{#if changed.includes(field) || (field === 'poster' && posterReplaced)}<span
											class="rounded-full bg-gold/15 px-1.5 text-[0.65rem] text-gold"
											>{t('admin.editor_changed')}</span
										>{/if}
								</label>
							{/each}
						</div>
					</div>

					{#if errorKey}
						<p role="alert" class="text-sm text-red-400 sm:col-span-2">{t(errorKey)}</p>
					{/if}

					<div class="flex justify-end gap-2 sm:col-span-2">
						<button
							type="button"
							disabled={saving}
							onclick={close}
							class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
						>
							{t('admin.cancel')}
						</button>
						<button
							type="submit"
							disabled={saving}
							class="rounded-md bg-gold px-3 py-1.5 text-sm font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
						>
							{saving ? t('admin.saving') : t('admin.save')}
						</button>
					</div>
				</form>
			{/if}
		</div>
	</div>
</div>
