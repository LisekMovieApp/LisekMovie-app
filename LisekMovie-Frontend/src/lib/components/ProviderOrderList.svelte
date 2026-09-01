<!--
  Faza 5 completion's "Drag & Drop Dostawców": lets an admin reorder, for
  one specific library, which metadata provider `admin::scan_library` (and
  the background refresh job) tries first for that library's items —
  overriding the admin's own global priority order (Admin Panel → Plugins)
  without changing it for every other library.

  Native HTML5 drag & drop rather than a library: this is a short,
  single-axis list and the project has no other drag-and-drop surface yet
  to justify a dependency for it.

  Persistence mirrors the backend's own "empty means global" contract (see
  `lisek_db::get_library_provider_order`'s doc comment): `savedOrder` is
  exactly what's stored for this library — empty until the admin actually
  drags something or a previous save wrote something — while `displayOrder`
  falls back to the admin's current global enabled/configured order purely
  for what the admin sees and drags. Dragging turns that fallback into a
  real, explicit saved order; "Use global order" clears it back to empty
  so this library keeps automatically following the admin's global order
  as it changes in the future, rather than freezing today's snapshot of it.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchProviderOrder, setProviderOrder } from '$lib/admin/mediaItems';
	import type { ProviderConfig } from '$lib/admin/providers';

	interface Props {
		libraryId: string;
		/** `"movie"`/`"episode"` — see `admin::item_type_for_library` on the
		 * backend for where this vocabulary comes from. */
		itemType: string;
		/** Every provider from the Admin Panel's Plugins tab — used both for
		 * display names and, filtered to enabled+configured, as the
		 * fallback order shown before this library has ever saved its own. */
		providers: ProviderConfig[];
	}
	let { libraryId, itemType, providers }: Props = $props();

	let savedOrder: string[] = $state([]);
	let loading = $state(true);
	let loadFailed = $state(false);
	let saving = $state(false);
	let saveFailed = $state(false);
	let dragIndex: number | null = $state(null);
	let overIndex: number | null = $state(null);

	const enabledProviders = $derived(providers.filter((p) => p.enabled && p.is_configured));
	const displayOrder = $derived(
		savedOrder.length > 0 ? savedOrder : enabledProviders.map((p) => p.provider_key)
	);

	function displayName(key: string): string {
		return providers.find((p) => p.provider_key === key)?.display_name ?? key;
	}

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		loading = true;
		loadFailed = false;
		try {
			savedOrder = await fetchProviderOrder(token, libraryId, itemType);
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	load();

	async function persist(next: string[]) {
		const token = getSessionCookieClientSide();
		if (!token) return;

		savedOrder = next;
		saving = true;
		saveFailed = false;
		try {
			await setProviderOrder(token, libraryId, itemType, next);
		} catch {
			saveFailed = true;
		} finally {
			saving = false;
		}
	}

	function handleDragStart(index: number, event: DragEvent) {
		dragIndex = index;
		event.dataTransfer?.setData('text/plain', String(index));
		if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move';
	}

	function handleDragOver(index: number, event: DragEvent) {
		event.preventDefault();
		if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
		overIndex = index;
	}

	function handleDrop(targetIndex: number, event: DragEvent) {
		event.preventDefault();
		const fromIndex = dragIndex;
		dragIndex = null;
		overIndex = null;
		if (fromIndex === null || fromIndex === targetIndex) return;

		const reordered = [...displayOrder];
		const [moved] = reordered.splice(fromIndex, 1);
		reordered.splice(targetIndex, 0, moved);
		void persist(reordered);
	}

	function handleDragEnd() {
		dragIndex = null;
		overIndex = null;
	}

	function resetToGlobal() {
		void persist([]);
	}
</script>

<div class="rounded-lg border border-white/10 bg-surface/60 p-4">
	<div class="flex flex-wrap items-start justify-between gap-2">
		<div>
			<h2 class="text-sm font-semibold text-neutral-100">{t('admin.provider_order_title')}</h2>
			<p class="mt-1 text-xs text-neutral-500">{t('admin.provider_order_hint')}</p>
		</div>
		{#if savedOrder.length > 0}
			<button
				type="button"
				onclick={resetToGlobal}
				disabled={saving}
				class="shrink-0 text-xs text-gold transition-colors hover:text-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
			>
				{t('admin.provider_order_reset')}
			</button>
		{/if}
	</div>

	{#if loading}
		<p class="mt-3 text-sm text-neutral-400">{t('admin.provider_order_loading')}</p>
	{:else if loadFailed}
		<p class="mt-3 text-sm text-red-400">{t('admin.provider_order_load_failed')}</p>
	{:else if displayOrder.length === 0}
		<p class="mt-3 text-sm text-neutral-400">{t('admin.provider_order_empty')}</p>
	{:else}
		<ul class="mt-3 flex flex-col gap-1.5">
			{#each displayOrder as providerKey, index (providerKey)}
				<li
					draggable="true"
					ondragstart={(event) => handleDragStart(index, event)}
					ondragover={(event) => handleDragOver(index, event)}
					ondrop={(event) => handleDrop(index, event)}
					ondragend={handleDragEnd}
					class="flex cursor-grab items-center gap-2 rounded-md border px-3 py-2 text-sm text-neutral-200 transition-colors active:cursor-grabbing {overIndex ===
						index && dragIndex !== index
						? 'border-gold bg-gold/5'
						: 'border-neutral-700 bg-black/40'}"
				>
					<span aria-hidden="true" class="text-neutral-600 select-none">⠿</span>
					<span class="text-xs text-neutral-500">{index + 1}.</span>
					<span>{displayName(providerKey)}</span>
				</li>
			{/each}
		</ul>
	{/if}

	{#if saveFailed}
		<p class="mt-2 text-sm text-red-400">{t('admin.provider_order_save_failed')}</p>
	{/if}
</div>
