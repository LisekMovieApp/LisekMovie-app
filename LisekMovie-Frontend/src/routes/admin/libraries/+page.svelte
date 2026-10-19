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

  Runda 9: "W górę"/"W dół" buttons on each card call the new
  `moveLibrary` — see `lisek_db::move_library`'s doc comment for exactly
  what it does server-side (a swap with the immediate neighbor's
  `sort_order`, inside one transaction). This is the ONE ordering every
  signed-in account's Navbar shares (`lisek-server::media::
  list_libraries`), not a per-admin preference, which is also why a
  successful move re-fetches the whole list from `load()` rather than
  reordering the local `libraries` array by hand — trusting the server's
  own `ORDER BY sort_order, name` to be the single source of truth is
  simpler and can't drift from what actually got written, at the cost of
  one extra round trip per click (this list is never large enough for that
  to matter).

  Scans run in the background on the server: "Skanuj" queues a job (202)
  and `watchScanJob` follows it — live `scan_progress` events over the
  notifications WebSocket, with a slow poll as fallback. Reopening this tab
  resumes showing any scan that is still running.
-->
<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchLibraries,
		deleteLibrary,
		startScan,
		fetchScanJobs,
		watchScanJob,
		isScanFinished,
		moveLibrary,
		type Library,
		type ScanJob,
		type ScanWatch
	} from '$lib/admin/libraries';
	import AddLibraryModal from '$lib/components/AddLibraryModal.svelte';

	let libraries: Library[] = $state([]);
	let loading = $state(true);
	let loadFailed = $state(false);
	let adding = $state(false);

	let busyLibraryId: string | null = $state(null);
	let scanFailedFor: string | null = $state(null);
	/** Latest known job per library id — running, or the last finished one. */
	let scanJobs: Record<string, ScanJob> = $state({});
	const watches: Record<string, ScanWatch> = {};
	let deleteFailedFor: string | null = $state(null);
	let confirmingDeleteId: string | null = $state(null);
	let reorderFailedFor: string | null = $state(null);

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

	onMount(() => {
		void load();
		void resumeActiveScans();
	});

	onDestroy(() => {
		for (const watch of Object.values(watches)) watch.stop();
	});

	function follow(token: string, job: ScanJob) {
		watches[job.library_id]?.stop();
		scanJobs[job.library_id] = job;
		const watch = watchScanJob(token, job, (update) => {
			scanJobs[update.library_id] = update;
		});
		watches[job.library_id] = watch;
		void watch.done.then((finished) => {
			if (watches[finished.library_id] === watch) delete watches[finished.library_id];
			if (finished.status === 'failed') scanFailedFor = finished.library_id;
		});
	}

	/** Shows scans that are still running from an earlier visit. */
	async function resumeActiveScans() {
		const token = getSessionCookieClientSide();
		if (!token) return;
		try {
			const seen: string[] = [];
			for (const job of await fetchScanJobs(token)) {
				if (seen.includes(job.library_id)) continue;
				seen.push(job.library_id);
				if (!isScanFinished(job)) follow(token, job);
			}
		} catch {
			// Optional: the page works without it.
		}
	}

	function isScanning(libraryId: string): boolean {
		const job = scanJobs[libraryId];
		return job !== undefined && !isScanFinished(job);
	}

	function scanProgressText(job: ScanJob): string {
		const params = { done: job.phase_done, total: job.phase_total };
		switch (job.phase) {
			case 'walking':
				return t('admin.scan_phase_walking', params);
			case 'saving':
				return t('admin.scan_phase_saving', params);
			case 'enriching':
				return t('admin.scan_phase_enriching', params);
			case 'probing':
				return t('admin.scan_phase_probing', params);
			default:
				return t('admin.scan_queued');
		}
	}

	function scanPercent(job: ScanJob): number {
		return job.phase_total > 0 ? Math.min(100, (job.phase_done / job.phase_total) * 100) : 0;
	}

	function handleCreated(library: Library) {
		libraries = [...libraries, library];
	}

	async function handleScan(library: Library) {
		const token = getSessionCookieClientSide();
		if (!token) return;

		busyLibraryId = library.id;
		scanFailedFor = null;
		try {
			follow(token, await startScan(token, library.id));
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
			watches[library.id]?.stop();
			delete watches[library.id];
			delete scanJobs[library.id];
		} catch {
			deleteFailedFor = library.id;
		} finally {
			busyLibraryId = null;
		}
	}

	async function handleMove(library: Library, direction: 'up' | 'down') {
		const token = getSessionCookieClientSide();
		if (!token) return;

		busyLibraryId = library.id;
		reorderFailedFor = null;
		try {
			await moveLibrary(token, library.id, direction);
			await load();
		} catch {
			reorderFailedFor = library.id;
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
				{#each libraries as library, i (library.id)}
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
								<!--
								  Runda 9: shown as a pair of arrow buttons rather than
								  drag&drop — a two-value swap-with-neighbor (see
								  `lisek_db::move_library`'s doc comment) doesn't need a
								  drop target to compute, unlike `ProviderOrderList`'s own
								  native HTML5 drag&drop, which reorders a whole list at
								  once. Disabled at each end of `libraries` (already sorted
								  by `sort_order` — this is the exact order `list_libraries`
								  returns) rather than relying on the backend's own no-op
								  boundary behavior to hide the buttons — a disabled button
								  reads immediately as "already first/last", not just inert.
								-->
								<button
									type="button"
									disabled={busyLibraryId === library.id || i === 0}
									onclick={() => handleMove(library, 'up')}
									aria-label={t('admin.move_up')}
									title={t('admin.move_up')}
									class="rounded-md border border-neutral-700 px-2 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-30"
								>
									↑
								</button>
								<button
									type="button"
									disabled={busyLibraryId === library.id || i === libraries.length - 1}
									onclick={() => handleMove(library, 'down')}
									aria-label={t('admin.move_down')}
									title={t('admin.move_down')}
									class="rounded-md border border-neutral-700 px-2 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-30"
								>
									↓
								</button>
								<a
									href={resolve('/admin/libraries/[id]/items', { id: library.id })}
									class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
								>
									{t('admin.browse_items')}
								</a>
								<button
									type="button"
									disabled={busyLibraryId === library.id || isScanning(library.id)}
									onclick={() => handleScan(library)}
									class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
								>
									{isScanning(library.id) ? t('admin.scanning') : t('admin.scan')}
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

						{#if scanJobs[library.id] && !isScanFinished(scanJobs[library.id])}
							{@const job = scanJobs[library.id]}
							<p class="mt-2 text-sm text-neutral-400" aria-live="polite">
								{scanProgressText(job)}
							</p>
							{#if job.phase_total > 0}
								<div class="mt-1 h-1 w-full overflow-hidden rounded bg-white/10">
									<div
										class="h-1 rounded bg-gold transition-[width]"
										style="width: {scanPercent(job)}%"
									></div>
								</div>
							{/if}
						{:else if scanJobs[library.id]?.status === 'completed'}
							{@const result = scanJobs[library.id]}
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
								{#if result.probed + result.probe_failed > 0}
									<p class="mt-1 text-sm text-neutral-400">
										{t('admin.scan_result_probe', {
											probed: result.probed,
											failed: result.probe_failed
										})}
									</p>
								{/if}
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
						{#if reorderFailedFor === library.id}
							<p class="mt-2 text-sm text-red-400">{t('admin.reorder_failed')}</p>
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
