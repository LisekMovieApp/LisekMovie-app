<!--
  Metadata editor (Faza 5, rebuilt in Faza 20). Two tabs: the text fields
  and the pictures.

  `scope` decides what is edited:
  - `item` — a film; an episode opened from the library browser is
    treated as `episode` by the server;
  - `series` — the series an episode belongs to: title, original title,
    year, overview and pictures for every episode at once;
  - `episode` — one episode's own title, overview, air date and still. The
    series title is shown, not edited (episodes are grouped by it).

  "Active fields" are the inverse of `locked_fields`: a checked field may
  still be updated by a rescan or a provider refresh. A field changed here
  is locked automatically on save (shown live), otherwise the next scan
  would take the title back from the file name. Replacing a picture locks
  it the same way.

  Faza 21: the score (0-10, stars on the pages) and the directors, for a
  film or series and for each episode on its own. Seasons have their own
  editor (`SeasonEditorModal`).

  Faza 22: "External ids" (film / series): the ids the item is matched
  with at TMDB, TheTVDB, IMDb …, editable. Scans ask the plugins by these
  ids only, never by the (possibly renamed) title. Saving changed ids
  fetches the metadata again by them; any other change locks the ids
  ("provider_ids" in the active fields) so a scan cannot re-identify the
  item. "Fetch again" refreshes the metadata now.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		editorErrorKey,
		fetchMediaItem,
		parseNames,
		parseRating,
		refreshMetadata,
		sameIds,
		updateMediaItem,
		validProviderId,
		ID_KINDS,
		ID_KIND_NAMES,
		type ArtworkKind,
		type EditScope,
		type MediaItemDetail,
		type UpdateMediaItemPayload
	} from '$lib/admin/mediaItems';
	import type { ProviderConfig } from '$lib/admin/providers';
	import { AGE_LIMITS } from '$lib/users/restrictions';
	import { showToast } from '$lib/toast/toast.svelte';
	import ArtworkEditor from './ArtworkEditor.svelte';
	import Icon from './Icon.svelte';

	const SHARED_FIELDS = [
		'title',
		'original_title',
		'year',
		'overview',
		'rating',
		'directors'
	] as const;
	const SHARED_ART: ArtworkKind[] = ['poster', 'backdrop', 'logo'];
	const EPISODE_FIELDS = [
		'episode_title',
		'episode_overview',
		'episode_rating',
		'episode_directors'
	] as const;

	interface Props {
		itemId: string;
		/** Every registered provider — used to populate the forced-provider
		 * dropdown (restricted to enabled+configured ones, plus whichever
		 * one this item is already forced to, even if that's since been
		 * disabled, so the editor never silently drops it on save). */
		providers: ProviderConfig[];
		/** What to edit; an episode defaults to itself. */
		scope?: EditScope;
		onClose: () => void;
		/** Called once the save succeeds, before `onClose`. */
		onSaved: () => void;
	}
	let { itemId, providers, scope: requestedScope = 'item', onClose, onSaved }: Props = $props();

	let loading = $state(true);
	let loadFailed = $state(false);
	let item = $state<MediaItemDetail | null>(null);
	let tab = $state<'details' | 'artwork'>('details');

	let title = $state('');
	let originalTitle = $state('');
	let year = $state('');
	let overview = $state('');
	let episodeTitle = $state('');
	let episodeOverview = $state('');
	let airDate = $state('');
	let rating = $state('');
	let directors = $state('');
	let episodeRating = $state('');
	let episodeDirectors = $state('');
	let metadataLocked = $state(false);
	let forcedProvider = $state('');
	/** Faza 6: '' = the provider's age rating applies. */
	let ageRating = $state('');
	let locked = $state<string[]>([]);
	/** Pictures replaced in this editor (locked by the server). */
	let replaced = $state<string[]>([]);
	let artChanged = false;
	/** Faza 22: the external ids, one row per kind. */
	let ids = $state<{ kind: string; value: string }[]>([]);
	let initialIds = $state<Record<string, string>>({});
	let addKind = $state('');
	let refreshing = $state(false);

	let saving = $state(false);
	let errorKey = $state<string | null>(null);

	const scope = $derived<EditScope>(
		item?.item_type === 'episode' ? (requestedScope === 'series' ? 'series' : 'episode') : 'item'
	);
	const episodeMode = $derived(scope === 'episode');
	const lockable = $derived<string[]>(
		episodeMode
			? [...EPISODE_FIELDS, 'thumb']
			: [...SHARED_FIELDS, ...SHARED_ART, 'provider_ids']
	);
	const artKinds = $derived<ArtworkKind[]>(episodeMode ? ['thumb'] : SHARED_ART);

	/** What the form had when it was loaded, to see which fields changed. */
	let initial = $state<Record<string, string>>({});
	const snapshot = () => ({
		title: title.trim(),
		original_title: originalTitle.trim(),
		// A number input's bound value is a number (or null) once edited.
		year: String(year ?? '').trim(),
		overview: overview.trim(),
		episode_title: episodeTitle.trim(),
		episode_overview: episodeOverview.trim(),
		rating: String(parseRating(rating) ?? rating.trim()),
		directors: parseNames(directors).join('\n'),
		episode_rating: String(parseRating(episodeRating) ?? episodeRating.trim()),
		episode_directors: parseNames(episodeDirectors).join('\n')
	});
	const current = $derived<Record<string, string>>(snapshot());
	const changed = $derived(
		lockable.filter((field) => field in current && current[field] !== initial[field])
	);
	const idRecord = $derived<Record<string, string>>(
		Object.fromEntries(ids.map((row) => [row.kind, row.value]))
	);
	const idsChanged = $derived(!sameIds(idRecord, initialIds));
	/** Any change locks the ids (when the item has some, or gets new ones). */
	const idsAutoLocked = $derived(
		(changed.length > 0 || replaced.length > 0 || idsChanged) &&
			(Object.keys(initialIds).length > 0 || idsChanged)
	);
	const autoLocked = (field: string) =>
		changed.includes(field) || (field === 'provider_ids' && idsAutoLocked);
	const isLocked = (field: string) =>
		locked.includes(field) || autoLocked(field) || replaced.includes(field);
	const addableKinds = $derived(ID_KINDS.filter((kind) => !ids.some((row) => row.kind === kind)));

	const forcedProviderOptions = $derived.by(() => {
		const options = providers.filter((p) => p.enabled && p.is_configured);
		if (item?.forced_provider && !options.some((p) => p.provider_key === item?.forced_provider)) {
			const stale = providers.find((p) => p.provider_key === item?.forced_provider);
			if (stale) options.push(stale);
		}
		return options;
	});

	const heading = $derived.by(() => {
		if (!item) return t('admin.editor_title');
		if (scope === 'series') return t('admin.editor_heading_series');
		if (episodeMode) return t('admin.editor_heading_episode');
		return t('admin.editor_heading_movie');
	});
	const subheading = $derived.by(() => {
		if (!item) return '';
		const series = item.series_title ?? item.title;
		if (episodeMode && item.season !== null && item.episode !== null)
			return t('admin.editor_episode_of', {
				series,
				code: `S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`
			});
		if (scope === 'series' && item.episode_count)
			return t('admin.editor_series_scope', { count: String(item.episode_count) });
		return item.title;
	});

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		loading = true;
		loadFailed = false;
		try {
			const detail = await fetchMediaItem(token, itemId);
			item = detail;
			title = (requestedScope === 'series' && detail.series_title) || detail.title;
			originalTitle = detail.original_title ?? '';
			year = detail.year !== null ? String(detail.year) : '';
			overview = detail.overview ?? '';
			episodeTitle = detail.episode_title ?? '';
			episodeOverview = detail.episode_overview ?? '';
			airDate = detail.episode_air_date ?? '';
			rating = detail.rating !== null && detail.rating !== undefined ? String(detail.rating) : '';
			directors = (detail.directors ?? []).join('\n');
			episodeRating =
				detail.episode_rating !== null && detail.episode_rating !== undefined
					? String(detail.episode_rating)
					: '';
			episodeDirectors = (detail.episode_directors ?? []).join('\n');
			metadataLocked = detail.metadata_locked;
			forcedProvider = detail.forced_provider ?? '';
			ageRating =
				detail.age_rating !== null && detail.age_rating !== undefined
					? String(detail.age_rating)
					: '';
			locked = [...detail.locked_fields];
			const stored = detail.provider_ids ?? {};
			const order = (kind: string) => {
				const at = (ID_KINDS as readonly string[]).indexOf(kind);
				return at < 0 ? ID_KINDS.length : at;
			};
			ids = Object.keys(stored)
				.sort((a, b) => order(a) - order(b))
				.map((kind) => ({ kind, value: stored[kind] }));
			initialIds = { ...stored };
			replaced = [];
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
			replaced = replaced.filter((f) => f !== field);
		} else {
			locked = [...locked, field];
		}
	}

	function lockedForSave(): string[] {
		return lockable.filter(
			(field) => locked.includes(field) || replaced.includes(field) || autoLocked(field)
		);
	}

	function addId() {
		if (!addKind || ids.some((row) => row.kind === addKind)) return;
		ids = [...ids, { kind: addKind, value: '' }];
		addKind = '';
	}

	async function fetchAgain() {
		const token = getSessionCookieClientSide();
		if (!token || refreshing) return;
		refreshing = true;
		errorKey = null;
		try {
			const result = await refreshMetadata(token, itemId);
			showToast(
				result.matched > 0 ? 'success' : 'warning',
				t('admin.editor_ids.refetched', {
					matched: String(result.matched),
					items: String(result.items)
				})
			);
			artChanged = true;
			await load();
		} catch (error) {
			errorKey = editorErrorKey(error);
		} finally {
			refreshing = false;
		}
	}

	async function handleSave(event: SubmitEvent) {
		event.preventDefault();
		const token = getSessionCookieClientSide();
		if (!token || !item) return;

		let payload: UpdateMediaItemPayload;
		const score = parseRating(episodeMode ? episodeRating : rating);
		if (score === undefined) {
			errorKey = 'admin.editor_errors.invalid_rating';
			return;
		}
		if (episodeMode) {
			if (airDate && !/^\d{4}-\d{2}-\d{2}$/.test(airDate)) {
				errorKey = 'admin.editor_errors.invalid_air_date';
				return;
			}
			payload = {
				scope: 'episode',
				episode_title: episodeTitle.trim() || null,
				episode_overview: episodeOverview.trim() || null,
				episode_air_date: airDate || null,
				episode_rating: score,
				episode_directors: parseNames(episodeDirectors),
				locked_fields: lockedForSave()
			};
		} else {
			const trimmedTitle = title.trim();
			if (!trimmedTitle) {
				errorKey = 'admin.editor_title_empty';
				return;
			}
			const trimmedYear = String(year ?? '').trim();
			const parsedYear = trimmedYear === '' ? null : Number(trimmedYear);
			if (ids.some((row) => row.value.trim() !== '' && !validProviderId(row.kind, row.value))) {
				errorKey = 'admin.editor_errors.invalid_provider_id';
				return;
			}
			payload = {
				scope,
				title: trimmedTitle,
				original_title: originalTitle.trim() || null,
				year: parsedYear !== null && Number.isFinite(parsedYear) ? parsedYear : null,
				overview: overview.trim() || null,
				metadata_locked: metadataLocked,
				forced_provider: forcedProvider || null,
				// Episode-level locks of the episodes are kept by the server.
				locked_fields: lockedForSave(),
				age_rating: ageRating === '' ? null : Number(ageRating),
				rating: score,
				directors: parseNames(directors),
				...(idsChanged ? { provider_ids: idRecord } : {})
			};
		}

		saving = true;
		errorKey = null;
		try {
			const result = await updateMediaItem(token, itemId, payload);
			showToast(
				'success',
				scope === 'series'
					? t('admin.editor_saved_series', { count: String(result.updated) })
					: t('admin.editor_saved')
			);
			if (result.refetched) {
				showToast(
					result.refetched.matched > 0 ? 'success' : 'warning',
					t('admin.editor_ids.refetched', {
						matched: String(result.refetched.matched),
						items: String(result.refetched.items)
					})
				);
			}
			onSaved();
			onClose();
		} catch (error) {
			errorKey = editorErrorKey(error);
		} finally {
			saving = false;
		}
	}

	function artworkChanged(kind: ArtworkKind) {
		artChanged = true;
		if (!replaced.includes(kind)) replaced = [...replaced, kind];
	}

	function close() {
		// A replaced picture is already saved: the page behind shows it.
		if (artChanged) onSaved();
		onClose();
	}

	function handleBackdropKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') close();
	}

	function handleBackdropClick(event: MouseEvent) {
		if (event.target === event.currentTarget) close();
	}

	const input =
		'rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold';
	const tabClass =
		'border-b-2 px-3 py-2 text-sm font-medium transition-colors aria-selected:border-gold aria-selected:text-white';
</script>

{#snippet externalIds()}
	<fieldset class="rounded-md border border-white/10 p-3">
		<legend class="px-1 text-sm font-semibold text-neutral-200">{t('admin.editor_ids.title')}</legend>
		<p class="text-xs text-neutral-400">{t('admin.editor_ids.hint')}</p>
		{#if ids.length === 0}
			<p class="mt-2 text-xs text-neutral-500">{t('admin.editor_ids.none')}</p>
		{/if}
		<div class="mt-2 flex flex-col gap-2">
			{#each ids as row (row.kind)}
				<div class="flex items-center gap-2">
					<label class="flex min-w-0 flex-1 items-center gap-2 text-sm text-neutral-300">
						<span class="w-28 shrink-0">{ID_KIND_NAMES[row.kind] ?? row.kind}</span>
						<input
							type="text"
							bind:value={row.value}
							maxlength="64"
							spellcheck="false"
							placeholder={t(
								row.kind === 'imdb' || row.kind === 'omdb'
									? 'admin.editor_ids.placeholder_imdb'
									: 'admin.editor_ids.placeholder_number'
							)}
							aria-invalid={row.value.trim() !== '' && !validProviderId(row.kind, row.value)}
							class="min-w-0 flex-1 font-mono text-sm aria-[invalid=true]:border-red-500 {input}"
						/>
					</label>
					<button
						type="button"
						aria-label={t('admin.editor_ids.remove', {
							service: ID_KIND_NAMES[row.kind] ?? row.kind
						})}
						onclick={() => (ids = ids.filter((other) => other.kind !== row.kind))}
						class="grid size-9 shrink-0 place-items-center rounded-md border border-neutral-700 text-neutral-300 hover:border-red-400 hover:text-red-300"
					>
						<Icon name="trash" class="size-4" />
					</button>
				</div>
			{/each}
		</div>
		<div class="mt-3 flex flex-wrap items-center gap-2">
			{#if addableKinds.length > 0}
				<select
					bind:value={addKind}
					aria-label={t('admin.editor_ids.add_label')}
					class="min-w-0 flex-1 rounded-md border border-neutral-700 bg-black px-2 py-1.5 text-sm text-neutral-100 focus:border-gold"
				>
					<option value="">{t('admin.editor_ids.add_placeholder')}</option>
					{#each addableKinds as kind (kind)}
						<option value={kind}>{ID_KIND_NAMES[kind]}</option>
					{/each}
				</select>
				<button
					type="button"
					disabled={!addKind}
					onclick={addId}
					class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50"
				>
					{t('admin.editor_ids.add')}
				</button>
			{/if}
			<button
				type="button"
				disabled={refreshing || saving || changed.length > 0 || idsChanged}
				title={changed.length > 0 || idsChanged ? t('admin.editor_ids.refresh_save_first') : undefined}
				onclick={fetchAgain}
				class="ml-auto flex items-center gap-1.5 rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50"
			>
				<Icon name="refresh" class="size-4" />
				{refreshing ? t('admin.editor_ids.refreshing') : t('admin.editor_ids.refresh')}
			</button>
		</div>
		{#if idsChanged}
			<p class="mt-2 text-xs text-gold">{t('admin.editor_ids.changed_hint')}</p>
		{/if}
	</fieldset>
{/snippet}

{#snippet scoreAndPeople(level: 'item' | 'episode')}
	<div class="grid gap-3 sm:grid-cols-[8rem_1fr]">
		<label class="flex flex-col gap-1 text-sm text-neutral-300">
			{t(level === 'episode' ? 'admin.editor_field_episode_rating' : 'admin.editor_field_rating')}
			{#if level === 'episode'}
				<input
					type="text"
					inputmode="decimal"
					bind:value={episodeRating}
					placeholder={t('admin.editor_rating_placeholder')}
					class={input}
				/>
			{:else}
				<input
					type="text"
					inputmode="decimal"
					bind:value={rating}
					placeholder={t('admin.editor_rating_placeholder')}
					class={input}
				/>
			{/if}
			<span class="text-xs text-neutral-500">{t('admin.editor_rating_hint')}</span>
		</label>
		<label class="flex flex-col gap-1 text-sm text-neutral-300">
			{t(
				level === 'episode' ? 'admin.editor_field_episode_directors' : 'admin.editor_field_directors'
			)}
			{#if level === 'episode'}
				<textarea bind:value={episodeDirectors} rows="2" class="resize-y {input}"></textarea>
			{:else}
				<textarea bind:value={directors} rows="2" class="resize-y {input}"></textarea>
			{/if}
			<span class="text-xs text-neutral-500">{t('admin.editor_directors_hint')}</span>
		</label>
	</div>
{/snippet}

<svelte:window onkeydown={handleBackdropKeydown} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
	onclick={handleBackdropClick}
>
	<div
		role="dialog"
		aria-modal="true"
		aria-labelledby="media-item-editor-title"
		class="flex max-h-[92vh] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-white/10 bg-surface"
	>
		<header class="border-b border-white/10 px-6 pt-5">
			<h2 id="media-item-editor-title" class="text-lg font-semibold text-neutral-100">
				{heading}
			</h2>
			{#if subheading}<p class="mt-0.5 truncate text-sm text-neutral-400">{subheading}</p>{/if}
			{#if item}
				<div role="tablist" class="mt-3 flex gap-1" aria-label={heading}>
					<button
						type="button"
						role="tab"
						aria-selected={tab === 'details'}
						class="{tabClass} {tab === 'details' ? '' : 'border-transparent text-neutral-400'}"
						onclick={() => (tab = 'details')}>{t('admin.editor_tab_details')}</button
					>
					<button
						type="button"
						role="tab"
						aria-selected={tab === 'artwork'}
						class="{tabClass} {tab === 'artwork' ? '' : 'border-transparent text-neutral-400'}"
						onclick={() => (tab = 'artwork')}>{t('admin.editor_tab_artwork')}</button
					>
				</div>
			{/if}
		</header>

		<div class="overflow-y-auto px-6 py-5">
			{#if loading}
				<p class="text-sm text-neutral-400">{t('admin.editor_loading')}</p>
			{:else if loadFailed || !item}
				<p class="text-sm text-red-400">{t('admin.editor_load_failed')}</p>
			{:else if tab === 'artwork'}
				<p class="mb-3 text-xs text-neutral-400">
					{scope === 'series' || (item.item_type === 'episode' && !episodeMode)
						? t('admin.editor_art.series_hint')
						: t('admin.editor_art.hint')}
				</p>
				<div class="grid gap-3 {episodeMode ? '' : 'sm:grid-cols-[1fr_2fr]'}">
					{#each artKinds as kind (kind)}
						<div class={kind === 'logo' ? 'sm:col-span-2' : ''}>
							<ArtworkEditor
								{itemId}
								{kind}
								version={item.version ?? 0}
								onchanged={artworkChanged}
							/>
						</div>
					{/each}
				</div>
				<div class="mt-4 flex justify-end">
					<button
						type="button"
						onclick={close}
						class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 hover:border-gold-dim"
						>{t('admin.editor_close')}</button
					>
				</div>
			{:else}
				<form onsubmit={handleSave} class="flex flex-col gap-3">
					{#if episodeMode}
						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{t('admin.editor_field_episode_title')}
							<input type="text" maxlength="500" bind:value={episodeTitle} class={input} />
						</label>
						<label class="flex w-44 flex-col gap-1 text-sm text-neutral-300">
							{t('admin.editor_field_air_date')}
							<input type="date" bind:value={airDate} class={input} />
						</label>
						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{t('admin.editor_field_episode_overview')}
							<textarea bind:value={episodeOverview} rows="5" class="resize-y {input}"></textarea>
						</label>
						{@render scoreAndPeople('episode')}
					{:else}
						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{scope === 'series'
								? t('admin.editor_field_series_title')
								: t('admin.editor_field_title')}
							<input type="text" maxlength="500" bind:value={title} class={input} />
							{#if scope === 'series'}<span class="text-xs text-neutral-500"
									>{t('admin.editor_series_title_hint')}</span
								>{/if}
						</label>

						<div class="grid gap-3 sm:grid-cols-[1fr_8rem]">
							<label class="flex flex-col gap-1 text-sm text-neutral-300">
								{t('admin.editor_field_original_title')}
								<input type="text" bind:value={originalTitle} class={input} />
							</label>
							<label class="flex flex-col gap-1 text-sm text-neutral-300">
								{t('admin.editor_field_year')}
								<input type="number" min="1800" max="2200" bind:value={year} class={input} />
							</label>
						</div>

						<label class="flex flex-col gap-1 text-sm text-neutral-300">
							{t('admin.editor_field_overview')}
							<textarea bind:value={overview} rows="4" class="resize-y {input}"></textarea>
						</label>
						{@render scoreAndPeople('item')}
						{@render externalIds()}

						<div class="grid gap-3 sm:grid-cols-2">
							<label class="flex flex-col gap-1 text-sm text-neutral-300">
								{t('admin.editor_forced_provider_label')}
								<select bind:value={forcedProvider} class={input}>
									<option value="">{t('admin.editor_forced_provider_none')}</option>
									{#each forcedProviderOptions as provider (provider.provider_key)}
										<option value={provider.provider_key}>{provider.display_name}</option>
									{/each}
								</select>
							</label>

							<label class="flex flex-col gap-1 text-sm text-neutral-300">
								{t('admin.editor_age_rating')}
								<select bind:value={ageRating} class={input}>
									<option value="">
										{item.provider_age_rating !== null && item.provider_age_rating !== undefined
											? t('admin.editor_age_from_provider', {
													label: item.certification ?? `${item.provider_age_rating}+`
												})
											: t('admin.editor_age_unrated')}
									</option>
									{#each AGE_LIMITS as age (age)}
										<option value={String(age)}>{`${age}+`}</option>
									{/each}
								</select>
							</label>
						</div>
						<span class="-mt-1 text-xs text-neutral-500">{t('admin.editor_age_rating_hint')}</span>

						<label class="flex items-start gap-2 text-sm text-neutral-300">
							<input type="checkbox" bind:checked={metadataLocked} class="mt-0.5 accent-gold" />
							<span>{t('admin.editor_lock_item_label')}</span>
						</label>
					{/if}

					<div class="mt-1 rounded-md border border-gold/30 bg-gold/5 p-3">
						<h3 class="text-sm font-semibold text-gold">{t('admin.editor_active_fields_title')}</h3>
						<p class="mt-1 text-xs text-neutral-400">{t('admin.editor_active_fields_hint')}</p>
						<div class="mt-3 grid gap-2 sm:grid-cols-2">
							{#each lockable as field (field)}
								<label class="flex items-center gap-2 text-sm text-neutral-200">
									<input
										type="checkbox"
										checked={!isLocked(field)}
										disabled={autoLocked(field)}
										onchange={() => toggleActive(field)}
										class="size-4 rounded border-neutral-600 bg-black accent-gold"
									/>
									{t(`admin.editor_field_${field}`)}
									{#if autoLocked(field) || replaced.includes(field)}<span
											class="rounded-full bg-gold/15 px-1.5 text-[0.65rem] text-gold"
											>{t('admin.editor_changed')}</span
										>{/if}
								</label>
							{/each}
						</div>
						{#if changed.length > 0}<p class="mt-2 text-xs text-neutral-400">
								{t('admin.editor_auto_lock_hint')}
							</p>{/if}
					</div>

					{#if errorKey}
						<p role="alert" class="text-sm text-red-400">{t(errorKey)}</p>
					{/if}

					<div class="mt-2 flex justify-end gap-2">
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
