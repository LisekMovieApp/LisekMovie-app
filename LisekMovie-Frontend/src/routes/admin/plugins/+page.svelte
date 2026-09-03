<!--
  Admin Panel — plugins tab: the metadata-provider registry (TMDB, TheTVDB)
  with enable/disable, a priority number, and a gear button opening
  `ProviderConfigModal` to set/clear an API key. Data loads client-side
  (via `onMount`) rather than through a `+page.ts` load function — this
  page's own `+layout.ts` guard already keeps non-admins out using data the
  root layout resolved server-side, but the provider list/mutations
  themselves reuse the same "get the token from the cookie, build the auth
  header" pattern `UserMenu.svelte`'s logout already established, rather
  than threading the session token through another layer of load functions
  just for this one page.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { fetchProviders, updateProvider, type ProviderConfig } from '$lib/admin/providers';
	import ProviderConfigModal from '$lib/components/ProviderConfigModal.svelte';

	let providers: ProviderConfig[] = $state([]);
	let loading = $state(true);
	let loadFailed = $state(false);
	let busyProviderKey: string | null = $state(null);
	let updateFailedFor: string | null = $state(null);
	let configuring: ProviderConfig | null = $state(null);

	async function load() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		loading = true;
		loadFailed = false;
		try {
			providers = await fetchProviders(token);
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	onMount(load);

	async function applyUpdate(provider: ProviderConfig, enabled: boolean, priority: number) {
		const token = getSessionCookieClientSide();
		if (!token) return;

		busyProviderKey = provider.provider_key;
		updateFailedFor = null;
		try {
			await updateProvider(token, provider.provider_key, enabled, priority);
			provider.enabled = enabled;
			provider.priority = priority;
		} catch {
			updateFailedFor = provider.provider_key;
		} finally {
			busyProviderKey = null;
		}
	}

	function handleToggle(provider: ProviderConfig) {
		applyUpdate(provider, !provider.enabled, provider.priority);
	}

	function handlePriorityInput(provider: ProviderConfig, event: Event) {
		const value = Number((event.target as HTMLInputElement).value);
		if (Number.isNaN(value)) return;
		applyUpdate(provider, provider.enabled, value);
	}
</script>

<main class="min-h-screen bg-base px-4 py-12 md:px-8">
	<div class="mx-auto max-w-2xl">
		<h1 class="text-2xl font-semibold text-neutral-100">{t('admin.plugins_title')}</h1>
		<p class="mt-1 text-sm text-neutral-400">{t('admin.plugins_subtitle')}</p>

		{#if loading}
			<p class="mt-8 text-sm text-neutral-400">{t('admin.loading')}</p>
		{:else if loadFailed}
			<p class="mt-8 text-sm text-red-400">{t('admin.load_failed')}</p>
		{:else}
			<ul class="mt-8 flex flex-col gap-3">
				{#each providers as provider (provider.provider_key)}
					<li class="rounded-lg border border-white/10 bg-surface/60 p-4">
						<div class="flex flex-wrap items-center justify-between gap-4">
							<div class="flex items-center gap-3">
								<span class="font-medium text-neutral-100">{provider.display_name}</span>
								<span
									class="rounded-full px-2 py-0.5 text-xs {provider.is_configured
										? 'bg-gold/15 text-gold'
										: 'bg-white/10 text-neutral-400'}"
								>
									{provider.is_configured
										? t('admin.status_configured')
										: t('admin.status_not_configured')}
								</span>
							</div>

							<div class="flex items-center gap-4">
								<label class="flex items-center gap-2 text-sm text-neutral-300">
									{t('admin.enabled')}
									<input
										type="checkbox"
										checked={provider.enabled}
										disabled={busyProviderKey === provider.provider_key}
										onchange={() => handleToggle(provider)}
										class="size-4 rounded border-neutral-600 bg-black text-gold accent-gold"
									/>
								</label>

								<label class="flex items-center gap-2 text-sm text-neutral-300">
									{t('admin.priority')}
									<input
										type="number"
										value={provider.priority}
										disabled={busyProviderKey === provider.provider_key}
										onchange={(event) => handlePriorityInput(provider, event)}
										class="w-16 rounded-md border border-neutral-700 bg-black px-2 py-1 text-neutral-100 focus:border-gold"
									/>
								</label>

								<button
									type="button"
									aria-label={t('admin.configure')}
									onclick={() => (configuring = provider)}
									class="rounded-md border border-neutral-700 p-1.5 text-neutral-300 transition-colors hover:border-gold-dim hover:text-gold"
								>
									<svg
										viewBox="0 0 24 24"
										fill="none"
										stroke="currentColor"
										stroke-width="2"
										class="size-4"
									>
										<path
											d="M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z"
											stroke-linecap="round"
											stroke-linejoin="round"
										/>
										<path
											d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"
											stroke-linecap="round"
											stroke-linejoin="round"
										/>
									</svg>
								</button>
							</div>
						</div>

						{#if updateFailedFor === provider.provider_key}
							<p class="mt-2 text-sm text-red-400">{t('admin.update_failed')}</p>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</main>

{#if configuring}
	<ProviderConfigModal provider={configuring} onClose={() => (configuring = null)} onSaved={load} />
{/if}
