<!--
  The "Add library" modal for the Admin Panel's libraries tab: a name, a
  type (movies/tv_shows), and one-or-more root-folder paths. Same
  backdrop/dialog/Escape-to-close shell as `ProviderConfigModal.svelte` —
  see that component's comments for why the backdrop needs a click handler
  and the panel doesn't.

  Root paths are entered as free-text, not browsed for: they're absolute
  paths on whatever machine `lisek-server` itself runs on (typically inside
  its Docker container, mounted from the host), which is very often not the
  machine running this browser — a folder picker here would let the admin
  pick a path that only makes sense on their own device. `root_paths_hint`
  says so explicitly.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { createLibrary, type Library, type LibraryType } from '$lib/admin/libraries';

	interface Props {
		onClose: () => void;
		/** Called with the newly created library once the POST succeeds, so
		 * the parent can append it to its list without a full re-fetch. */
		onCreated: (library: Library) => void;
	}

	let { onClose, onCreated }: Props = $props();

	let name = $state('');
	let libraryType = $state<LibraryType>('movies');
	let rootPaths = $state<string[]>(['']);
	// Matches the backend's own default (see the Faza 5 round-2 migration
	// and `CreateLibraryRequest`'s serde default) — most self-hosters
	// expect provider images written directly next to their media.
	let saveMetadataInMediaFolder = $state(true);
	let saving = $state(false);
	let errorKey = $state<string | null>(null);

	function addPathField() {
		rootPaths = [...rootPaths, ''];
	}

	function removePathField(index: number) {
		if (rootPaths.length <= 1) return;
		rootPaths = rootPaths.filter((_, i) => i !== index);
	}

	async function handleSave(event: SubmitEvent) {
		event.preventDefault();

		const trimmedName = name.trim();
		const trimmedPaths = rootPaths.map((path) => path.trim()).filter((path) => path.length > 0);

		if (!trimmedName) {
			errorKey = 'admin.name_empty';
			return;
		}
		if (trimmedPaths.length === 0) {
			errorKey = 'admin.root_path_empty';
			return;
		}

		const token = getSessionCookieClientSide();
		if (!token) return;

		saving = true;
		errorKey = null;
		try {
			const library = await createLibrary(
				token,
				trimmedName,
				libraryType,
				trimmedPaths,
				saveMetadataInMediaFolder
			);
			onCreated(library);
			onClose();
		} catch {
			errorKey = 'admin.create_failed';
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
		aria-labelledby="add-library-title"
		class="w-full max-w-md rounded-lg border border-white/10 bg-surface p-6"
	>
		<h2 id="add-library-title" class="mb-4 text-lg font-semibold text-neutral-100">
			{t('admin.add_library')}
		</h2>

		<form onsubmit={handleSave} class="flex flex-col gap-3">
			<label class="flex flex-col gap-1 text-sm text-neutral-300">
				{t('admin.library_name_label')}
				<input
					type="text"
					bind:value={name}
					placeholder={t('admin.library_name_placeholder')}
					class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
				/>
			</label>

			<label class="flex flex-col gap-1 text-sm text-neutral-300">
				{t('admin.library_type_label')}
				<select
					bind:value={libraryType}
					class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
				>
					<option value="movies">{t('admin.library_type_movies')}</option>
					<option value="tv_shows">{t('admin.library_type_tv_shows')}</option>
				</select>
			</label>

			<div class="flex flex-col gap-1 text-sm text-neutral-300">
				<span>{t('admin.root_paths_label')}</span>
				<p class="text-xs text-neutral-500">{t('admin.root_paths_hint')}</p>

				<div class="mt-1 flex flex-col gap-2">
					{#each rootPaths as _, index (index)}
						<div class="flex gap-2">
							<input
								type="text"
								bind:value={rootPaths[index]}
								placeholder={t('admin.root_path_placeholder')}
								class="flex-1 rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
							/>
							<button
								type="button"
								aria-label={t('admin.remove_path')}
								disabled={rootPaths.length <= 1}
								onclick={() => removePathField(index)}
								class="rounded-md border border-neutral-700 px-2 text-neutral-400 transition-colors hover:border-gold-dim hover:text-gold disabled:cursor-not-allowed disabled:opacity-30"
							>
								&times;
							</button>
						</div>
					{/each}
				</div>

				<button
					type="button"
					onclick={addPathField}
					class="mt-1 self-start text-sm text-gold transition-colors hover:text-gold-dim"
				>
					+ {t('admin.add_path')}
				</button>
			</div>

			<label class="flex items-start gap-2 text-sm text-neutral-300">
				<input
					type="checkbox"
					bind:checked={saveMetadataInMediaFolder}
					class="mt-0.5 accent-gold"
				/>
				<span class="flex flex-col gap-0.5">
					<span>{t('admin.save_metadata_in_media_folder_label')}</span>
					<span class="text-xs text-neutral-500">
						{t('admin.save_metadata_in_media_folder_hint')}
					</span>
				</span>
			</label>

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
					{saving ? t('admin.creating') : t('admin.save')}
				</button>
			</div>
		</form>
	</div>
</div>
