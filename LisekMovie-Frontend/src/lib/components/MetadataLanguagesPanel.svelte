<!--
  Faza 21: the metadata language order, on the plugins tab. The scanner
  asks every plugin for the first language; an empty synopsis or title is
  fetched again in the next one ("Język oryginalny" is the production's own
  language, "Dowolny" whatever is left). Up/down buttons reorder the list;
  saving an empty list goes back to the default order.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		addEntry,
		fetchMetadataLanguages,
		languageLabelKey,
		MAX_LANGUAGES,
		moveEntry,
		remainingChoices,
		saveMetadataLanguages
	} from '$lib/admin/metadataLanguages';
	import { showToast } from '$lib/toast/toast.svelte';
	import Icon from './Icon.svelte';

	let order = $state<string[]>([]);
	let saved = $state<string[]>([]);
	let isDefault = $state(true);
	let loading = $state(true);
	let failed = $state(false);
	let saving = $state(false);
	let errorKey = $state<string | null>(null);
	let picked = $state('');

	const choices = $derived(remainingChoices(order));
	const dirty = $derived(order.join(',') !== saved.join(','));

	function apply(answer: { languages: string[]; effective: string[] }) {
		order = [...answer.effective];
		saved = [...answer.effective];
		isDefault = answer.languages.length === 0;
	}

	onMount(async () => {
		const token = getSessionCookieClientSide();
		if (!token) return;
		try {
			apply(await fetchMetadataLanguages(token));
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	});

	async function save(languages: string[]) {
		const token = getSessionCookieClientSide();
		if (!token) return;
		saving = true;
		errorKey = null;
		try {
			apply(await saveMetadataLanguages(token, languages));
			showToast('success', t('admin.metadata_languages.saved'));
		} catch {
			errorKey = 'admin.metadata_languages.save_failed';
		} finally {
			saving = false;
		}
	}

	function add() {
		order = addEntry(order, picked);
		picked = '';
	}

	const iconButton =
		'grid size-8 place-items-center rounded-md text-neutral-300 hover:bg-white/10 hover:text-gold focus-visible:outline-2 focus-visible:outline-gold disabled:opacity-30';
</script>

<section
	class="mt-8 rounded-lg border border-white/10 bg-surface/60 p-4"
	aria-labelledby="metadata-languages-title"
>
	<h2 id="metadata-languages-title" class="font-medium text-neutral-100">
		{t('admin.metadata_languages.title')}
	</h2>
	<p class="mt-1 text-xs text-neutral-400">{t('admin.metadata_languages.hint')}</p>

	{#if loading}
		<p class="mt-3 text-sm text-neutral-400">{t('admin.loading')}</p>
	{:else if failed}
		<p class="mt-3 text-sm text-red-400">{t('admin.load_failed')}</p>
	{:else}
		{#if isDefault}
			<p class="mt-2 text-xs text-neutral-500">{t('admin.metadata_languages.default_note')}</p>
		{/if}
		<ol class="mt-3 flex flex-col gap-1.5">
			{#each order as code, index (code)}
				<li class="flex items-center gap-2 rounded-md bg-black/30 px-2 py-1.5 ring-1 ring-white/5">
					<span class="w-6 text-center text-sm font-semibold text-gold">{index + 1}.</span>
					<span class="flex-1 text-sm text-neutral-100">{t(languageLabelKey(code))}</span>
					<button
						type="button"
						class={iconButton}
						disabled={index === 0 || saving}
						aria-label={t('admin.metadata_languages.move_up', {
							language: t(languageLabelKey(code))
						})}
						onclick={() => (order = moveEntry(order, index, -1))}
					>
						<Icon name="chevron_left" class="size-4 rotate-90" />
					</button>
					<button
						type="button"
						class={iconButton}
						disabled={index === order.length - 1 || saving}
						aria-label={t('admin.metadata_languages.move_down', {
							language: t(languageLabelKey(code))
						})}
						onclick={() => (order = moveEntry(order, index, 1))}
					>
						<Icon name="chevron_right" class="size-4 rotate-90" />
					</button>
					<button
						type="button"
						class={iconButton}
						disabled={order.length === 1 || saving}
						aria-label={t('admin.metadata_languages.remove', {
							language: t(languageLabelKey(code))
						})}
						onclick={() => (order = order.filter((entry) => entry !== code))}
					>
						<Icon name="close" class="size-4" />
					</button>
				</li>
			{/each}
		</ol>

		{#if choices.length > 0 && order.length < MAX_LANGUAGES}
			<div class="mt-3 flex gap-2">
				<select
					bind:value={picked}
					aria-label={t('admin.metadata_languages.add_label')}
					class="min-w-0 flex-1 rounded-md border border-neutral-700 bg-black px-2 py-1.5 text-sm text-neutral-100 focus:border-gold"
				>
					<option value="">{t('admin.metadata_languages.add_placeholder')}</option>
					{#each choices as code (code)}
						<option value={code}>{t(languageLabelKey(code))}</option>
					{/each}
				</select>
				<button
					type="button"
					disabled={!picked || saving}
					onclick={add}
					class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50"
				>
					{t('admin.metadata_languages.add')}
				</button>
			</div>
		{/if}

		{#if errorKey}<p role="alert" class="mt-2 text-sm text-red-400">{t(errorKey)}</p>{/if}

		<div class="mt-4 flex flex-wrap justify-end gap-2">
			{#if !isDefault}
				<button
					type="button"
					disabled={saving}
					onclick={() => save([])}
					class="rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50"
				>
					{t('admin.metadata_languages.reset')}
				</button>
			{/if}
			<button
				type="button"
				disabled={!dirty || saving}
				onclick={() => save(order)}
				class="rounded-md bg-gold px-3 py-1.5 text-sm font-medium text-black hover:bg-gold-dim disabled:opacity-50"
			>
				{saving ? t('admin.saving') : t('admin.save')}
			</button>
		</div>
	{/if}
</section>
