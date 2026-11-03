<!--
  Faza 25: "Edytuj" on the libraries tab — a library's name, its folders
  and whether artwork is saved next to the media. A new disk is one more
  folder ("Dodaj ścieżkę"); the next scan walks every folder. Removing a
  folder takes its items out of the library (the files stay on disk), so
  the modal says how many folders go before saving. Folders are paths on
  the server's machine, typed, not picked (see `AddLibraryModal`). A live
  TV library has a name only; its sources have their own panel.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { AdminApiError } from '$lib/admin/api';
	import {
		normalizeRoot,
		removedRoots,
		updateLibrary,
		type Library,
		type LibraryUpdateResult
	} from '$lib/admin/libraries';
	import Icon from './Icon.svelte';

	interface Props {
		library: Library;
		onClose: () => void;
		onSaved: (result: LibraryUpdateResult) => void;
	}

	let { library, onClose, onSaved }: Props = $props();

	const liveTv = $derived(library.library_type === 'live_tv');
	let name = $state('');
	let rootPaths = $state<string[]>(['']);
	let saveInMediaFolder = $state(true);
	let saving = $state(false);
	let errorKey = $state<string | null>(null);

	$effect.pre(() => {
		name = library.name;
		rootPaths = library.root_paths.length > 0 ? [...library.root_paths] : [''];
		saveInMediaFolder = library.save_metadata_in_media_folder;
	});

	const typed = $derived(rootPaths.map((path) => path.trim()).filter((path) => path !== ''));
	const invalid = $derived(typed.filter((path) => normalizeRoot(path) === null));
	const removed = $derived(removedRoots(library.root_paths, typed));

	function addPath() {
		rootPaths = [...rootPaths, ''];
	}

	function removePath(index: number) {
		rootPaths = rootPaths.length <= 1 ? [''] : rootPaths.filter((_, i) => i !== index);
	}

	async function handleSave(event: SubmitEvent) {
		event.preventDefault();
		const trimmedName = name.trim();
		if (!trimmedName) {
			errorKey = 'admin.name_empty';
			return;
		}
		if (!liveTv && typed.length === 0) {
			errorKey = 'admin.root_path_empty';
			return;
		}
		if (!liveTv && invalid.length > 0) {
			errorKey = 'admin.edit_library.path_invalid';
			return;
		}
		const token = getSessionCookieClientSide();
		if (!token) return;
		saving = true;
		errorKey = null;
		try {
			const result = await updateLibrary(
				token,
				library.id,
				liveTv
					? { name: trimmedName }
					: {
							name: trimmedName,
							root_paths: typed,
							save_metadata_in_media_folder: saveInMediaFolder
						}
			);
			onSaved(result);
			onClose();
		} catch (error) {
			errorKey =
				error instanceof AdminApiError && error.status === 400
					? 'admin.edit_library.path_invalid'
					: 'admin.edit_library.failed';
		} finally {
			saving = false;
		}
	}

	const input =
		'rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold';
</script>

<svelte:window onkeydown={(event) => event.key === 'Escape' && onClose()} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
	onclick={(event) => event.target === event.currentTarget && onClose()}
>
	<div
		role="dialog"
		aria-modal="true"
		aria-labelledby="edit-library-title"
		class="max-h-[92vh] w-full max-w-lg overflow-y-auto rounded-lg border border-white/10 bg-surface p-6"
	>
		<h2 id="edit-library-title" class="mb-4 text-lg font-semibold text-neutral-100">
			{t('admin.edit_library.title')}
		</h2>

		<form onsubmit={handleSave} class="flex flex-col gap-4">
			<label class="flex flex-col gap-1 text-sm text-neutral-300">
				{t('admin.library_name_label')}
				<input type="text" maxlength="200" bind:value={name} class={input} />
			</label>

			{#if !liveTv}
				<fieldset class="flex flex-col gap-1 text-sm text-neutral-300">
					<legend>{t('admin.root_paths_label')}</legend>
					<p class="text-xs text-neutral-500">{t('admin.edit_library.paths_hint')}</p>
					<div class="mt-1 flex flex-col gap-2">
						{#each rootPaths as path, index (index)}
							{@const bad = path.trim() !== '' && normalizeRoot(path) === null}
							<div class="flex gap-2">
								<input
									type="text"
									bind:value={rootPaths[index]}
									placeholder={t('admin.root_path_placeholder')}
									aria-label={t('admin.edit_library.path_label', { number: String(index + 1) })}
									aria-invalid={bad}
									spellcheck="false"
									class="min-w-0 flex-1 font-mono text-sm {bad ? 'border-red-500' : ''} {input}"
								/>
								<button
									type="button"
									aria-label={t('admin.edit_library.remove_path', {
										path: path.trim() || String(index + 1)
									})}
									onclick={() => removePath(index)}
									class="grid w-10 shrink-0 place-items-center rounded-md border border-neutral-700 text-neutral-400 transition-colors hover:border-red-400 hover:text-red-300"
								>
									<Icon name="trash" class="size-4" />
								</button>
							</div>
						{/each}
					</div>
					<button
						type="button"
						onclick={addPath}
						class="mt-1 flex items-center gap-1 self-start text-sm text-gold transition-colors hover:text-gold-dim"
					>
						<Icon name="plus" class="size-4" />
						{t('admin.add_path')}
					</button>
				</fieldset>

				{#if removed.length > 0}
					<div
						role="note"
						class="rounded-md border border-amber-400/40 bg-amber-400/10 p-3 text-xs text-amber-200"
					>
						<p>{t('admin.edit_library.removed_warning', { count: String(removed.length) })}</p>
						<ul class="mt-1 list-inside list-disc font-mono">
							{#each removed as path (path)}<li class="truncate">{path}</li>{/each}
						</ul>
					</div>
				{/if}

				<label class="flex items-start gap-2 text-sm text-neutral-300">
					<input type="checkbox" bind:checked={saveInMediaFolder} class="mt-0.5 accent-gold" />
					<span class="flex flex-col gap-0.5">
						<span>{t('admin.save_metadata_in_media_folder_label')}</span>
						<span class="text-xs text-neutral-500"
							>{t('admin.save_metadata_in_media_folder_hint')}</span
						>
					</span>
				</label>
			{/if}

			{#if errorKey}
				<p role="alert" class="text-sm text-red-400">{t(errorKey)}</p>
			{/if}

			<div class="flex justify-end gap-2">
				<button
					type="button"
					disabled={saving}
					onclick={onClose}
					class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:opacity-50"
				>
					{t('admin.cancel')}
				</button>
				<button
					type="submit"
					disabled={saving}
					class="rounded-md bg-gold px-3 py-1.5 text-sm font-medium text-black transition-colors hover:bg-gold-dim disabled:opacity-50"
				>
					{saving ? t('admin.saving') : t('admin.save')}
				</button>
			</div>
		</form>
	</div>
</div>
