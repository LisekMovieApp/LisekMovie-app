<!--
  Admin Panel — API tab (Faza 25): named keys for scripts and automations
  (Discord / Telegram bots, Home Assistant, own dashboards). A new key is
  shown once, with a ready `curl` line; the list shows each key's name,
  prefix, creation date and when it was last used (updated by the server
  at most once a minute), so unused keys are easy to spot and revoke.
  Keys replace the single bots' key that used to live under Statistics;
  that key kept working and appears here as "Statystyki (bot)".
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		API_KEY_HEADER,
		SUMMARY_ENDPOINT,
		createApiKey,
		curlExample,
		deleteApiKey,
		fetchApiKeys,
		renameApiKey,
		validKeyName,
		type ApiKey,
		type NewApiKey
	} from '$lib/admin/apiKeys';
	import Icon from '$lib/components/Icon.svelte';

	let keys = $state<ApiKey[]>([]);
	let loading = $state(true);
	let loadFailed = $state(false);
	let name = $state('');
	let creating = $state(false);
	let errorKey = $state<string | null>(null);
	let fresh = $state<NewApiKey | null>(null);
	let copied = $state<'key' | 'curl' | null>(null);
	let confirmingId = $state<string | null>(null);
	let renamingId = $state<string | null>(null);
	let renameValue = $state('');
	let busyId = $state<string | null>(null);

	const origin = typeof location === 'undefined' ? '' : location.origin;
	const example = $derived(curlExample(origin, fresh?.key ?? t('api_keys.key_placeholder')));

	function token(): string {
		return getSessionCookieClientSide() ?? '';
	}

	async function load() {
		loading = true;
		loadFailed = false;
		try {
			keys = await fetchApiKeys(token());
		} catch {
			loadFailed = true;
		} finally {
			loading = false;
		}
	}

	onMount(load);

	async function create(event: SubmitEvent) {
		event.preventDefault();
		if (!validKeyName(name)) {
			errorKey = 'api_keys.name_invalid';
			return;
		}
		creating = true;
		errorKey = null;
		copied = null;
		try {
			fresh = await createApiKey(token(), name.trim());
			name = '';
			keys = [fresh, ...keys];
		} catch {
			errorKey = 'api_keys.create_failed';
		} finally {
			creating = false;
		}
	}

	async function copy(what: 'key' | 'curl') {
		if (!fresh) return;
		try {
			await navigator.clipboard.writeText(what === 'key' ? fresh.key : example);
			copied = what;
		} catch {
			copied = null;
		}
	}

	async function revoke(key: ApiKey) {
		if (confirmingId !== key.id) {
			confirmingId = key.id;
			return;
		}
		busyId = key.id;
		errorKey = null;
		try {
			await deleteApiKey(token(), key.id);
			keys = keys.filter((k) => k.id !== key.id);
			if (fresh?.id === key.id) fresh = null;
		} catch {
			errorKey = 'api_keys.delete_failed';
		} finally {
			busyId = null;
			confirmingId = null;
		}
	}

	function startRename(key: ApiKey) {
		renamingId = key.id;
		renameValue = key.name;
		confirmingId = null;
	}

	async function saveRename(key: ApiKey) {
		if (!validKeyName(renameValue)) {
			errorKey = 'api_keys.name_invalid';
			return;
		}
		busyId = key.id;
		errorKey = null;
		try {
			await renameApiKey(token(), key.id, renameValue.trim());
			keys = keys.map((k) => (k.id === key.id ? { ...k, name: renameValue.trim() } : k));
			renamingId = null;
		} catch {
			errorKey = 'api_keys.rename_failed';
		} finally {
			busyId = null;
		}
	}

	const when = (iso: string) =>
		new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
	const button =
		'rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:opacity-50';
</script>

<main class="min-h-screen bg-base px-4 py-12 md:px-8">
	<div class="mx-auto max-w-4xl">
		<h1 class="text-2xl font-semibold text-neutral-100">{t('api_keys.title')}</h1>
		<p class="mt-1 text-sm text-neutral-400">
			{t('api_keys.subtitle', { header: API_KEY_HEADER })}
		</p>

		<form
			onsubmit={create}
			class="mt-6 flex flex-col gap-2 rounded-lg border border-white/10 bg-surface/60 p-4 sm:flex-row sm:items-end"
		>
			<label class="flex min-w-0 flex-1 flex-col gap-1 text-sm text-neutral-300">
				{t('api_keys.name_label')}
				<input
					type="text"
					bind:value={name}
					maxlength="100"
					placeholder={t('api_keys.name_placeholder')}
					class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
				/>
			</label>
			<button
				type="submit"
				disabled={creating}
				class="flex items-center justify-center gap-1.5 rounded-md bg-gold px-4 py-2 text-sm font-medium text-black transition-colors hover:bg-gold-dim disabled:opacity-50"
			>
				<Icon name="key" class="size-4" />
				{creating ? t('api_keys.creating') : t('api_keys.create')}
			</button>
		</form>

		{#if errorKey}<p role="alert" class="mt-3 text-sm text-red-400">{t(errorKey)}</p>{/if}

		{#if fresh}
			<section
				class="mt-4 rounded-lg bg-black/40 p-4 ring-1 ring-gold/40"
				aria-labelledby="new-key-title"
			>
				<h2 id="new-key-title" class="text-sm font-semibold text-gold">
					{t('api_keys.new_title', { name: fresh.name })}
				</h2>
				<p class="mt-1 text-xs text-neutral-400">{t('api_keys.new_hint')}</p>
				<div class="mt-3 flex flex-wrap items-center gap-2">
					<code class="min-w-0 flex-1 rounded bg-white/5 px-2 py-1.5 text-xs break-all"
						>{fresh.key}</code
					>
					<button type="button" class={button} onclick={() => copy('key')}>
						{copied === 'key' ? t('api_keys.copied') : t('api_keys.copy')}
					</button>
				</div>
				<p class="mt-3 mb-1 text-xs text-neutral-400">{t('api_keys.example')}</p>
				<div class="flex flex-wrap items-start gap-2">
					<pre
						class="min-w-0 flex-1 overflow-x-auto rounded bg-black/40 p-3 text-xs text-neutral-300">{example}</pre>
					<button type="button" class={button} onclick={() => copy('curl')}>
						{copied === 'curl' ? t('api_keys.copied') : t('api_keys.copy')}
					</button>
				</div>
			</section>
		{/if}

		<section class="mt-8" aria-labelledby="keys-title">
			<h2 id="keys-title" class="mb-3 text-lg font-semibold text-neutral-100">
				{t('api_keys.list_title')}
			</h2>
			{#if loading}
				<p class="text-sm text-neutral-400">{t('admin.loading')}</p>
			{:else if loadFailed}
				<p class="text-sm text-red-400">{t('admin.load_failed')}</p>
			{:else if keys.length === 0}
				<p class="text-sm text-neutral-400">{t('api_keys.none')}</p>
			{:else}
				<div class="overflow-x-auto rounded-lg ring-1 ring-white/10">
					<table class="w-full text-left text-sm">
						<thead class="bg-white/5 text-neutral-300">
							<tr>
								<th class="p-3 font-medium">{t('api_keys.col_name')}</th>
								<th class="p-3 font-medium">{t('api_keys.col_key')}</th>
								<th class="p-3 font-medium whitespace-nowrap">{t('api_keys.col_created')}</th>
								<th class="p-3 font-medium whitespace-nowrap">{t('api_keys.col_last_used')}</th>
								<th class="p-3"><span class="sr-only">{t('api_keys.col_actions')}</span></th>
							</tr>
						</thead>
						<tbody>
							{#each keys as key (key.id)}
								<tr class="border-t border-white/5 align-middle">
									<td class="p-3 text-neutral-100">
										{#if renamingId === key.id}
											<input
												type="text"
												bind:value={renameValue}
												maxlength="100"
												aria-label={t('api_keys.rename_label')}
												class="w-full min-w-40 rounded-md border border-neutral-700 bg-black px-2 py-1 text-sm text-neutral-100 focus:border-gold"
												onkeydown={(event) => {
													if (event.key === 'Enter') {
														event.preventDefault();
														void saveRename(key);
													} else if (event.key === 'Escape') {
														event.stopPropagation();
														renamingId = null;
													}
												}}
											/>
										{:else}
											{key.name}
										{/if}
									</td>
									<td class="p-3 font-mono text-xs whitespace-nowrap text-neutral-400"
										>{key.prefix ? `${key.prefix}…` : '—'}</td
									>
									<td class="p-3 whitespace-nowrap text-neutral-300">{when(key.created_at)}</td>
									<td class="p-3 whitespace-nowrap">
										{#if key.last_used_at}
											<span class="text-neutral-200">{when(key.last_used_at)}</span>
										{:else}
											<span class="text-neutral-500">{t('api_keys.never_used')}</span>
										{/if}
									</td>
									<td class="p-3">
										<div class="flex justify-end gap-2">
											{#if renamingId === key.id}
												<button
													type="button"
													class={button}
													disabled={busyId === key.id}
													onclick={() => saveRename(key)}>{t('admin.save')}</button
												>
												<button type="button" class={button} onclick={() => (renamingId = null)}
													>{t('admin.cancel')}</button
												>
											{:else}
												<button
													type="button"
													class={button}
													disabled={busyId === key.id}
													onclick={() => startRename(key)}>{t('api_keys.rename')}</button
												>
												<button
													type="button"
													disabled={busyId === key.id}
													onclick={() => revoke(key)}
													class="rounded-md border px-3 py-1.5 text-sm whitespace-nowrap transition-colors disabled:opacity-50 {confirmingId ===
													key.id
														? 'border-red-400 bg-red-400/10 text-red-300'
														: 'border-neutral-700 text-neutral-200 hover:border-red-400 hover:text-red-300'}"
												>
													{confirmingId === key.id
														? t('api_keys.confirm_revoke')
														: t('api_keys.revoke')}
												</button>
											{/if}
										</div>
									</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
		</section>

		<section
			class="mt-8 rounded-lg border border-white/10 bg-surface/60 p-4 text-sm text-neutral-300"
		>
			<h2 class="font-semibold text-neutral-100">{t('api_keys.usage_title')}</h2>
			<p class="mt-1 text-neutral-400">{t('api_keys.usage_hint')}</p>
			<ul class="mt-2 list-inside list-disc text-neutral-400">
				<li>
					<code class="text-neutral-200">{SUMMARY_ENDPOINT}</code>
					— {t('api_keys.usage_summary')}
				</li>
			</ul>
		</section>
	</div>
</main>
