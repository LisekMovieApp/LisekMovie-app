<!--
  Admin Panel — libraries tab: create/list/delete libraries and trigger a
  scan for each. Mirrors `/admin/plugins/+page.svelte`'s data-loading shape
  (client-side `onMount` fetch using the session cookie, rather than a
  `+page.ts` load function — see that page's top comment for why).

  Delete is destructive (cascades to every `media_items` row the library
  owns — see `lisek-db::delete_library`), so it uses a two-click inline
  confirm rather than either a native `confirm()` dialog or a whole extra
  modal component: click once to arm it, click the now-red "confirm" state
  to actually delete, or click anywhere else on the card list to disarm it.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchLibraries,
		deleteLibrary,
		scanLibrary,
		type Library,
		type ScanSummary
	} from '$lib/admin/libraries';
	import AddLibraryModal from '$lib/components/AddLibraryModal.svelte';

	let libraries: Library[] = $state([]);
	let loading = $state(true);
	let loadFailed = $state(false);
	let adding = $state(false);

	let busyLibraryId: string | null = $state(null);
	let scanFailedFor: string | null = $state(null);
	let scanResults: Record<string, ScanSummary> = $state({});
	let deleteFailedFor: string | null = $state(null);
	let confirmingDeleteId: string | null = $state(null);

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		loading = true;
		loadFailed = false;
		try {
			libraries = await fetchLibraries(token);
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	onMount(load);

	function handleCreated(library: Library) {
		libraries = [...libraries, library];
	}

	async function handleScan(library: Library) {
		const token = getSessionCookieClientSide();
		if (!token) return;

		busyLibraryId = library.id;
		scanFailedFor = null;
		delete scanResults[library.id];
		try {
			scanResults[library.id] = await scanLibrary(token, library.id);
		} catch {
			scanFailedFor = library.id;
		} finally {
			busyLibraryId = null;
		}
	}

	function handleDeleteClick(library: Library) {
		if (confirmingDeleteId === library.id) {
			void handleDeleteConfirmed(library);
		} else {
			confirmingDeleteId = library.id;
		}
	}

	async function handleDeleteConfirmed(library: Library) {
		const token = getSessionCookieClientSide();
		if (!token) return;

		busyLibraryId = library.id;
		deleteFailedFor = null;
		confirmingDeleteId = null;
		try {
			await deleteLibrary(token, library.id);
			libraries = libraries.filter((l) => l.id !== library.id);
			delete scanResults[library.id];
		} catch {
			deleteFailedFor = library.id;
		} finally {
			busyLibraryId = null;
		}
	}
</script>

<main class="min-h-screen bg-base px-4 py-12 md:px-8">
	<div class="mx-auto max-w-2xl">
		<div class="flex flex-wrap items-start justify-between gap-4">
			<div>
				<h1 class="text-2xl font-semibold text-neutral-100">{t('admin.libraries_title')}</h1>
				<p class="mt-1 text-sm text-neutral-400">{t('admin.libraries_subtitle')}</p>
			</div>
			<button
				type="button"
				onclick={() => (adding = true)}
				class="rounded-md bg-gold px-3 py-1.5 text-sm font-medium whitespace-nowrap text-black transition-colors hover:bg-gold-dim"
			>
				+ {t('admin.add_library')}
			</button>
		</div>

		{#if loading}
			<p class="mt-8 text-sm text-neutral-400">{t('admin.libraries_loading')}</p>
		{:else if loadFailed}
			<p class="mt-8 text-sm text-red-400">{t('admin.libraries_load_failed')}</p>
		{:else if libraries.length === 0}
			<p class="mt-8 text-sm text-neutral-400">{t('admin.no_libraries')}</p>
		{:else}
			<ul class="mt-8 flex flex-col gap-3">
				{#each libraries as library (library.id)}
					<li class="rounded-lg border border-white/10 bg-surface/60 p-4">
						<div class="flex flex-wrap items-center justify-between gap-4">
							<div class="min-w-0">
								<div class="flex items-center gap-2">
									<span class="font-medium text-neutral-100">{library.name}</span>
									<span class="rounded-full bg-white/10 px-2 py-0.5 text-xs text-neutral-400">
										{library.library_type === 'movies'
											? t('admin.library_type_movies')
											: t('admin.library_type_tv_shows')}
									</span>
								</div>
								<p class="mt-1 truncate text-xs text-neutral-500">
									{library.root_paths.join(', ')}
								</p>
							</div>

							<div class="flex items-center gap-2">
								<a
									href={resolve('/admin/libraries/[id]/items', { id: library.id })}
									class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
								>
									{t('admin.browse_items')}
								</a>
								<button
									type="button"
									disabled={busyLibraryId === library.id}
									onclick={() => handleScan(library)}
									class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
								>
									{busyLibraryId === library.id && scanFailedFor !== library.id
										? t('admin.scanning')
										: t('admin.scan')}
								</button>
								<button
									type="button"
									disabled={busyLibraryId === library.id}
									onclick={() => handleDeleteClick(library)}
									class="rounded-md border px-3 py-1.5 text-sm transition-colors disabled:cursor-not-allowed disabled:opacity-50 {confirmingDeleteId ===
									library.id
										? 'border-red-400 bg-red-400/10 text-red-300'
										: 'border-neutral-700 text-neutral-200 hover:border-red-400 hover:text-red-300'}"
								>
									{confirmingDeleteId === library.id
										? t('admin.confirm_delete')
										: t('admin.delete')}
								</button>
							</div>
						</div>

						{#if scanResults[library.id]}
							{@const result = scanResults[library.id]}
							{@const totalItems = result.movies_found + result.episodes_found}
							<p class="mt-2 text-sm text-neutral-400">
								{t('admin.scan_result', {
									movies: result.movies_found,
									episodes: result.episodes_found,
									unrecognized: result.unrecognized
								})}
							</p>
							{#if totalItems > 0}
								<p class="mt-1 text-sm text-neutral-400">
									{t('admin.scan_result_images', {
										matched: result.metadata_matched,
										saved: result.images_saved,
										total: totalItems
									})}
								</p>
								{#if result.images_saved === 0}
									<p class="mt-1 text-sm text-amber-400">
										{t('admin.scan_no_images_hint')}
									</p>
								{/if}
							{/if}
						{/if}
						{#if scanFailedFor === library.id}
							<p class="mt-2 text-sm text-red-400">{t('admin.scan_failed')}</p>
						{/if}
						{#if deleteFailedFor === library.id}
							<p class="mt-2 text-sm text-red-400">{t('admin.delete_failed')}</p>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</main>

{#if adding}
	<AddLibraryModal onClose={() => (adding = false)} onCreated={handleCreated} />
{/if}
