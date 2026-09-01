<!--
  Faza 5 completion's "Zaawansowany Edytor Metadanych" — reproduces the
  functional layout of a reference screenshot the admin supplied (a form
  with title/original title/year/description plus a per-field "active
  fields" lock section), rebuilt from scratch with this project's own
  Tailwind classes and gold/surface palette — no class names or markup from
  that reference were reused.

  Same backdrop/dialog/Escape-to-close shell as `AddLibraryModal.svelte`/
  `ProviderConfigModal.svelte`.

  "Pola aktywne" (active fields) is the admin-facing framing of
  `locked_fields`: every field starts checked (active — a future scan or
  the background refresh job may still update it); unchecking one adds it
  to `locked_fields` on save, protecting it from exactly those two paths
  (see `lisek_db::attach_provider_metadata` / `upsert_media_item` for where
  that's actually enforced) without locking the whole item. `metadata_locked`
  (the "Zablokuj tę pozycję" checkbox) is the coarser, whole-item version of
  the same idea, already enforced since Faza 1 — see `upsert_media_item`'s
  own doc comment. Scoped to exactly the 4 fields this editor exposes
  (title, original_title, year, overview) — not "Genres" (no such concept
  exists anywhere in this schema) and not a "date added" field (that's
  `created_at`, a system timestamp, not metadata an admin edits).
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchMediaItem,
		updateMediaItem,
		type MediaItemDetail,
		type UpdateMediaItemPayload
	} from '$lib/admin/mediaItems';
	import type { ProviderConfig } from '$lib/admin/providers';

	const LOCKABLE_FIELDS = ['title', 'original_title', 'year', 'overview'] as const;
	type LockableField = (typeof LOCKABLE_FIELDS)[number];

	interface Props {
		itemId: string;
		/** Every registered provider — used to populate the forced-provider
		 * dropdown (restricted to enabled+configured ones, plus whichever
		 * one this item is already forced to, even if that's since been
		 * disabled, so the editor never silently drops it on save). */
		providers: ProviderConfig[];
		onClose: () => void;
		/** Called once the save succeeds, before `onClose` — the parent
		 * (the library items browse page) re-fetches its own list rather
		 * than this modal trying to hand back a full patched row. */
		onSaved: () => void;
	}
	let { itemId, providers, onClose, onSaved }: Props = $props();

	let loading = $state(true);
	let loadFailed = $state(false);
	let item = $state<MediaItemDetail | null>(null);

	let title = $state('');
	let originalTitle = $state('');
	let year = $state('');
	let overview = $state('');
	let metadataLocked = $state(false);
	let forcedProvider = $state('');
	let activeFields = $state<LockableField[]>([...LOCKABLE_FIELDS]);

	let saving = $state(false);
	let errorKey = $state<string | null>(null);

	const forcedProviderOptions = $derived.by(() => {
		const options = providers.filter((p) => p.enabled && p.is_configured);
		if (item?.forced_provider && !options.some((p) => p.provider_key === item?.forced_provider)) {
			const stale = providers.find((p) => p.provider_key === item?.forced_provider);
			if (stale) options.push(stale);
		}
		return options;
	});

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		loading = true;
		loadFailed = false;
		try {
			const detail = await fetchMediaItem(token, itemId);
			item = detail;
			title = detail.title;
			originalTitle = detail.original_title ?? '';
			year = detail.year !== null ? String(detail.year) : '';
			overview = detail.overview ?? '';
			metadataLocked = detail.metadata_locked;
			forcedProvider = detail.forced_provider ?? '';
			activeFields = LOCKABLE_FIELDS.filter((field) => !detail.locked_fields.includes(field));
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	load();

	function toggleActiveField(field: LockableField) {
		activeFields = activeFields.includes(field)
			? activeFields.filter((f) => f !== field)
			: [...activeFields, field];
	}

	async function handleSave(event: SubmitEvent) {
		event.preventDefault();

		const trimmedTitle = title.trim();
		if (!trimmedTitle) {
			errorKey = 'admin.editor_title_empty';
			return;
		}

		const token = getSessionCookieClientSide();
		if (!token) return;

		const trimmedYear = year.trim();
		const parsedYear = trimmedYear === '' ? null : Number(trimmedYear);
		const payload: UpdateMediaItemPayload = {
			title: trimmedTitle,
			original_title: originalTitle.trim() || null,
			year: parsedYear !== null && Number.isFinite(parsedYear) ? parsedYear : null,
			overview: overview.trim() || null,
			metadata_locked: metadataLocked,
			forced_provider: forcedProvider || null,
			locked_fields: LOCKABLE_FIELDS.filter((field) => !activeFields.includes(field))
		};

		saving = true;
		errorKey = null;
		try {
			await updateMediaItem(token, itemId, payload);
			onSaved();
			onClose();
		} catch {
			errorKey = 'admin.editor_save_failed';
		} finally {
			saving = false;
		}
	}

	function handleBackdropKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') onClose();
	}

	function handleBackdropClick(event: MouseEvent) {
		if (event.target === event.currentTarget) onClose();
	}
</script>

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
		class="max-h-[90vh] w-full max-w-lg overflow-y-auto rounded-lg border border-white/10 bg-surface p-6"
	>
		<h2 id="media-item-editor-title" class="mb-4 text-lg font-semibold text-neutral-100">
			{t('admin.editor_title')}
		</h2>

		{#if loading}
			<p class="text-sm text-neutral-400">{t('admin.editor_loading')}</p>
		{:else if loadFailed}
			<p class="text-sm text-red-400">{t('admin.editor_load_failed')}</p>
		{:else}
			<form onsubmit={handleSave} class="flex flex-col gap-3">
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('admin.editor_field_title')}
					<input
						type="text"
						bind:value={title}
						class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					/>
				</label>

				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('admin.editor_field_original_title')}
					<input
						type="text"
						bind:value={originalTitle}
						class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					/>
				</label>

				<label class="flex w-32 flex-col gap-1 text-sm text-neutral-300">
					{t('admin.editor_field_year')}
					<input
						type="number"
						bind:value={year}
						class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					/>
				</label>

				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('admin.editor_field_overview')}
					<textarea
						bind:value={overview}
						rows="4"
						class="resize-y rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					></textarea>
				</label>

				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('admin.editor_forced_provider_label')}
					<select
						bind:value={forcedProvider}
						class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					>
						<option value="">{t('admin.editor_forced_provider_none')}</option>
						{#each forcedProviderOptions as provider (provider.provider_key)}
							<option value={provider.provider_key}>{provider.display_name}</option>
						{/each}
					</select>
				</label>

				<label class="flex items-start gap-2 text-sm text-neutral-300">
					<input type="checkbox" bind:checked={metadataLocked} class="mt-0.5 accent-gold" />
					<span>{t('admin.editor_lock_item_label')}</span>
				</label>

				<div class="mt-1 rounded-md border border-gold/30 bg-gold/5 p-3">
					<h3 class="text-sm font-semibold text-gold">{t('admin.editor_active_fields_title')}</h3>
					<p class="mt-1 text-xs text-neutral-400">{t('admin.editor_active_fields_hint')}</p>

					<div class="mt-3 flex flex-col gap-2">
						{#each LOCKABLE_FIELDS as field (field)}
							<label class="flex items-center gap-2 text-sm text-neutral-200">
								<input
									type="checkbox"
									checked={activeFields.includes(field)}
									onchange={() => toggleActiveField(field)}
									class="size-4 rounded border-neutral-600 bg-black accent-gold"
								/>
								{t(`admin.editor_field_${field}`)}
							</label>
						{/each}
					</div>
				</div>

				{#if errorKey}
					<p class="text-sm text-red-400">{t(errorKey)}</p>
				{/if}

				<div class="mt-2 flex justify-end gap-2">
					<button
						type="button"
						disabled={saving}
						onclick={onClose}
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
