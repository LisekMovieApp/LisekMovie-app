<!--
  Admin Panel — one library's items browse view: Faza 5 completion's entry
  point into the advanced metadata editor (`MediaItemEditorModal`), reached
  from the libraries tab's new "Przeglądaj" button on each library card.
  Also hosts this library's provider-order drag & drop (`ProviderOrderList`)
  at the top — that feature is inherently per-library (see
  `admin::item_type_for_library`), so it lives here rather than getting its
  own separate route.

  Same client-side `onMount` fetch shape as `/admin/libraries` and
  `/admin/plugins` — see either page's own top comment for why this project
  doesn't use a `+page.ts` load function for admin data.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchLibraries, type Library } from '$lib/admin/libraries';
	import { fetchProviders, type ProviderConfig } from '$lib/admin/providers';
	import { fetchLibraryItems, type MediaItemSummary } from '$lib/admin/mediaItems';
	import ProviderOrderList from '$lib/components/ProviderOrderList.svelte';
	import MediaItemEditorModal from '$lib/components/MediaItemEditorModal.svelte';

	const libraryId = $derived(page.params.id ?? '');
	let library = $state<Library | null>(null);
	// Mirrors `admin::item_type_for_library` on the backend exactly — kept
	// in sync manually since it's a two-value mapping, not worth a shared
	// generated constant for.
	const itemType = $derived(library?.library_type === 'movies' ? 'movie' : 'episode');

	let providers: ProviderConfig[] = $state([]);
	let items: MediaItemSummary[] = $state([]);
	let loading = $state(true);
	let loadFailed = $state(false);
	let editingItemId: string | null = $state(null);

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		loading = true;
		loadFailed = false;
		try {
			const [libraries, fetchedProviders, fetchedItems] = await Promise.all([
				fetchLibraries(token),
				fetchProviders(token),
				fetchLibraryItems(token, libraryId)
			]);
			library = libraries.find((l) => l.id === libraryId) ?? null;
			providers = fetchedProviders;
			items = fetchedItems;
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	onMount(load);

	async function reloadItems() {
		const token = getSessionCookieClientSide();
		if (!token) return;
		try {
			items = await fetchLibraryItems(token, libraryId);
		} catch {
			// A failed refresh after a successful save just leaves the list
			// stale until the admin reloads the page — not worth its own
			// error banner on top of the editor modal's already having
			// closed successfully.
		}
	}
</script>

<main class="min-h-screen bg-base px-4 py-12 md:px-8">
	<div class="mx-auto max-w-2xl">
		<a
			href={resolve('/admin/libraries')}
			class="text-sm text-neutral-400 transition-colors hover:text-gold"
		>
			{t('admin.back_to_libraries')}
		</a>

		<h1 class="mt-2 text-2xl font-semibold text-neutral-100">
			{t('admin.items_title', { name: library?.name ?? '…' })}
		</h1>
		<p class="mt-1 text-sm text-neutral-400">{t('admin.items_subtitle')}</p>

		{#if loading}
			<p class="mt-8 text-sm text-neutral-400">{t('admin.items_loading')}</p>
		{:else if loadFailed}
			<p class="mt-8 text-sm text-red-400">{t('admin.items_load_failed')}</p>
		{:else}
			<div class="mt-6">
				<ProviderOrderList {libraryId} {itemType} {providers} />
			</div>

			{#if items.length === 0}
				<p class="mt-8 text-sm text-neutral-400">{t('admin.no_items')}</p>
			{:else}
				<ul class="mt-6 flex flex-col gap-2">
					{#each items as item (item.id)}
						<li
							class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-white/10 bg-surface/60 p-3"
						>
							<div class="flex min-w-0 items-center gap-2">
								<span class="truncate text-sm font-medium text-neutral-100">
									{item.title}{item.year ? ` (${item.year})` : ''}
								</span>
								{#if item.season !== null && item.episode !== null}
									<span class="shrink-0 text-xs text-neutral-500">
										{t('admin.episode_code', { season: item.season ?? '', episode: item.episode ?? '' })}
									</span>
								{/if}
								{#if item.metadata_locked}
									<span
										class="shrink-0 rounded-full bg-white/10 px-2 py-0.5 text-xs text-neutral-400"
									>
										🔒 {t('admin.item_locked_badge')}
									</span>
								{/if}
								{#if item.metadata_fallback}
									<span
										class="shrink-0 rounded-full bg-amber-400/15 px-2 py-0.5 text-xs text-amber-300"
									>
										{t('admin.item_fallback_badge')}
									</span>
								{/if}
							</div>

							<button
								type="button"
								onclick={() => (editingItemId = item.id)}
								class="shrink-0 rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim"
							>
								{t('admin.edit_item')}
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		{/if}
	</div>
</main>

{#if editingItemId}
	<MediaItemEditorModal
		itemId={editingItemId}
		{providers}
		onClose={() => (editingItemId = null)}
		onSaved={reloadItems}
	/>
{/if}
