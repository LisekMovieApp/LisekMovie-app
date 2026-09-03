<!--
  Admin Panel — "Widok Ostrzeżeń Fallback": every item currently flagged
  `metadata_fallback` (its poster/backdrop only exist in the server's local
  cache, not next to its video file — see `metadata_enrichment::enrich_item`'s
  own doc comment for when that happens), each with a one-click action that
  calls `admin::migrate_fallback_item` to try copying the cached image(s)
  across now that the underlying folder-permission problem may be fixed.

  Same client-side `onMount` fetch shape as every other admin page in this
  project (see `/admin/libraries/+page.svelte`'s top comment for why).
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchFallbackItems,
		migrateFallbackItem,
		type FallbackItem,
		type MigrateFallbackResult
	} from '$lib/admin/mediaItems';

	let items: FallbackItem[] = $state([]);
	let loading = $state(true);
	let loadFailed = $state(false);

	let busyItemId: string | null = $state(null);
	let migrateFailedFor: string | null = $state(null);
	let migrateResults: Record<string, MigrateFallbackResult> = $state({});

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		loading = true;
		loadFailed = false;
		try {
			items = await fetchFallbackItems(token);
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	onMount(load);

	async function handleMigrate(item: FallbackItem) {
		const token = getSessionCookieClientSide();
		if (!token) return;

		busyItemId = item.id;
		migrateFailedFor = null;
		delete migrateResults[item.id];
		try {
			const result = await migrateFallbackItem(token, item.id);
			migrateResults[item.id] = result;
			if (result.poster_migrated || result.backdrop_migrated) {
				// A fully-migrated item no longer needs to be on this list at
				// all; a partial migration (one of the two images) stays —
				// the result banner below explains what's still missing.
				if (result.poster_migrated && result.backdrop_migrated) {
					items = items.filter((i) => i.id !== item.id);
				}
			}
		} catch {
			migrateFailedFor = item.id;
		} finally {
			busyItemId = null;
		}
	}

	function migrateResultKey(result: MigrateFallbackResult): string {
		if (result.poster_migrated && result.backdrop_migrated) return 'admin.fallback_migrated_both';
		if (result.poster_migrated) return 'admin.fallback_migrated_poster_only';
		if (result.backdrop_migrated) return 'admin.fallback_migrated_backdrop_only';
		return 'admin.fallback_migrate_nothing';
	}
</script>

<main class="min-h-screen bg-base px-4 py-12 md:px-8">
	<div class="mx-auto max-w-2xl">
		<h1 class="text-2xl font-semibold text-neutral-100">{t('admin.fallback_title')}</h1>
		<p class="mt-1 text-sm text-neutral-400">{t('admin.fallback_subtitle')}</p>

		{#if loading}
			<p class="mt-8 text-sm text-neutral-400">{t('admin.fallback_loading')}</p>
		{:else if loadFailed}
			<p class="mt-8 text-sm text-red-400">{t('admin.fallback_load_failed')}</p>
		{:else if items.length === 0}
			<p class="mt-8 text-sm text-neutral-400">{t('admin.no_fallback_items')}</p>
		{:else}
			<ul class="mt-8 flex flex-col gap-3">
				{#each items as item (item.id)}
					<li class="rounded-lg border border-white/10 bg-surface/60 p-4">
						<div class="flex flex-wrap items-center justify-between gap-4">
							<div class="min-w-0">
								<span class="font-medium text-neutral-100">{item.title}</span>
								<p class="mt-1 truncate text-xs text-neutral-500">{item.file_path}</p>
							</div>

							<button
								type="button"
								disabled={busyItemId === item.id}
								onclick={() => handleMigrate(item)}
								class="shrink-0 rounded-md bg-gold px-3 py-1.5 text-sm font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
							>
								{busyItemId === item.id
									? t('admin.fallback_migrating')
									: t('admin.fallback_migrate')}
							</button>
						</div>

						{#if migrateResults[item.id]}
							{@const result = migrateResults[item.id]}
							<p
								class="mt-2 text-sm {result.poster_migrated && result.backdrop_migrated
									? 'text-emerald-400'
									: 'text-amber-400'}"
							>
								{t(migrateResultKey(result))}
							</p>
						{/if}
						{#if migrateFailedFor === item.id}
							<p class="mt-2 text-sm text-red-400">{t('admin.fallback_migrate_failed')}</p>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</main>
