<!--
  The gear-icon modal for one metadata provider's Admin Panel row: enter/
  replace its API key, or remove the stored one. Deliberately never fetches
  or displays the current key — the backend has no "read the key back"
  endpoint at all (see `lisek-server/src/admin.rs`'s doc comment on
  `set_provider_key`), so this modal only ever writes, never reads, one.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		setProviderApiKey,
		clearProviderApiKey,
		type ProviderConfig
	} from '$lib/admin/providers';

	interface Props {
		provider: ProviderConfig;
		onClose: () => void;
		/** Called after a successful save or clear, so the parent can re-fetch
		 * the list and pick up the new `is_configured` state. */
		onSaved: () => void;
	}

	let { provider, onClose, onSaved }: Props = $props();

	let apiKey = $state('');
	let saving = $state(false);
	let clearing = $state(false);
	let errorKey = $state<string | null>(null);

	let busy = $derived(saving || clearing);

	async function handleSave(event: SubmitEvent) {
		event.preventDefault();
		const trimmed = apiKey.trim();
		if (!trimmed) {
			errorKey = 'admin.api_key_empty';
			return;
		}

		const token = getSessionCookieClientSide();
		if (!token) return;

		saving = true;
		errorKey = null;
		try {
			await setProviderApiKey(token, provider.provider_key, trimmed);
			onSaved();
			onClose();
		} catch {
			errorKey = 'admin.save_failed';
		} finally {
			saving = false;
		}
	}

	async function handleClearKey() {
		const token = getSessionCookieClientSide();
		if (!token) return;

		clearing = true;
		errorKey = null;
		try {
			await clearProviderApiKey(token, provider.provider_key);
			onSaved();
			onClose();
		} catch {
			errorKey = 'admin.save_failed';
		} finally {
			clearing = false;
		}
	}

	function handleBackdropKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') onClose();
	}

	// Closes only for a click on the backdrop itself, never one that bubbled
	// up from inside the panel — checking `target === currentTarget` here
	// (rather than a `stopPropagation` click handler on the panel) means the
	// panel element needs no click handler of its own at all, so it isn't
	// flagged as a click-interactive element needing a matching keyboard
	// handler and a tabindex it has no real use for.
	function handleBackdropClick(event: MouseEvent) {
		if (event.target === event.currentTarget) onClose();
	}
</script>

<svelte:window onkeydown={handleBackdropKeydown} />

<!--
  role="presentation" + a click handler here is the standard modal-backdrop
  pattern (click outside the panel to dismiss); see `handleBackdropClick`
  for why the panel itself needs no click handler to make that work.
-->
<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
	onclick={handleBackdropClick}
>
	<div
		role="dialog"
		aria-modal="true"
		aria-labelledby="provider-config-title"
		class="w-full max-w-sm rounded-lg border border-white/10 bg-surface p-6"
	>
		<h2 id="provider-config-title" class="mb-2 text-lg font-semibold text-neutral-100">
			{t('admin.config_modal_title', { provider: provider.display_name })}
		</h2>
		<p class="mb-1 text-sm text-neutral-400">{t(`admin.provider_hint.${provider.provider_key}`)}</p>
		<p class="mb-4 text-xs text-neutral-500">
			{t(`admin.key_requirement.${provider.key_requirement}`)}
			{#if provider.website}
				·
				<!-- eslint-disable svelte/no-navigation-without-resolve -- external site -->
				<a
					href={provider.website}
					target="_blank"
					rel="noopener noreferrer"
					class="text-gold underline-offset-2 hover:underline">{t('admin.get_key')}</a
				>
				<!-- eslint-enable svelte/no-navigation-without-resolve -->
			{/if}
		</p>

		<form onsubmit={handleSave} class="flex flex-col gap-3">
			<label class="flex flex-col gap-1 text-sm text-neutral-300">
				{t('admin.api_key_label')}
				<input
					type="password"
					bind:value={apiKey}
					placeholder={t('admin.api_key_placeholder')}
					autocomplete="off"
					class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
				/>
			</label>

			{#if errorKey}
				<p class="text-sm text-red-400">{t(errorKey)}</p>
			{/if}

			<div class="mt-2 flex items-center justify-between gap-2">
				{#if provider.key_stored}
					<button
						type="button"
						disabled={busy}
						onclick={handleClearKey}
						class="text-sm text-red-400 transition-colors hover:text-red-300 disabled:cursor-not-allowed disabled:opacity-50"
					>
						{clearing ? t('admin.clearing') : t('admin.clear_key')}
					</button>
				{:else}
					<span></span>
				{/if}

				<div class="flex gap-2">
					<button
						type="button"
						disabled={busy}
						onclick={onClose}
						class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
					>
						{t('admin.cancel')}
					</button>
					<button
						type="submit"
						disabled={busy}
						class="rounded-md bg-gold px-3 py-1.5 text-sm font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
					>
						{saving ? t('admin.saving') : t('admin.save')}
					</button>
				</div>
			</div>
		</form>
	</div>
</div>
