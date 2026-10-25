<!--
  Admin → Konta → (account) → "Kontrola dostępu" (Faza 6, parental
  controls). Limits what one account can browse and play:

  - which libraries it sees (all, or a chosen few — e.g. only "Bajki"),
  - the highest age rating (from TMDB certifications: PG-13 → 13, FSK 12
    → 12, TV-MA → 17), optionally also hiding unrated items,
  - quick profiles: child (up to 7, unrated hidden), teenager (up to 13),
    no limits.

  Saved separately from the profile form (`PUT /api/admin/users/{id}/
  restrictions`); takes effect on the account's next request.
  Administrators always see everything: the section says so and stays
  read-only for them.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import Icon from './Icon.svelte';
	import type { Library } from '$lib/admin/libraries';
	import {
		AGE_LIMITS,
		PRESETS,
		getRestrictions,
		isRestricted,
		saveRestrictions,
		toRequest
	} from '$lib/users/restrictions';
	import { accountErrorKey } from '$lib/users/api';

	interface Props {
		userId: string;
		isAdmin: boolean;
		libraries: Library[];
		token: string;
		disabled?: boolean;
		/** Reports whether the account is restricted after a load or save. */
		onchange?: (restricted: boolean) => void;
	}

	let { userId, isAdmin, libraries, token, disabled = false, onchange }: Props = $props();

	let loaded = $state(false);
	let busy = $state(false);
	let error = $state('');
	let saved = $state(false);
	let allLibraries = $state(true);
	let chosen = $state<string[]>([]);
	let maxAge = $state<number | null>(null);
	let blockUnrated = $state(false);

	async function load(id: string) {
		loaded = false;
		error = '';
		saved = false;
		try {
			const current = await getRestrictions(token, id);
			if (id !== userId) return;
			allLibraries = current.libraries === null || current.libraries === undefined;
			chosen = current.libraries ?? [];
			maxAge = current.max_age_rating ?? null;
			blockUnrated = current.block_unrated ?? false;
			loaded = true;
			onchange?.(current.enforced && isRestricted(current));
		} catch (e) {
			error = accountErrorKey(e);
		}
	}

	$effect(() => {
		const id = userId;
		void load(id);
	});

	function toggle(libraryId: string, on: boolean) {
		chosen = on ? [...new Set([...chosen, libraryId])] : chosen.filter((id) => id !== libraryId);
		saved = false;
	}

	function preset(name: keyof typeof PRESETS) {
		maxAge = PRESETS[name].max_age_rating;
		blockUnrated = PRESETS[name].block_unrated;
		if (name === 'none') allLibraries = true;
		saved = false;
	}

	async function save(event: SubmitEvent) {
		event.preventDefault();
		if (busy) return;
		busy = true;
		error = '';
		saved = false;
		try {
			const result = await saveRestrictions(
				token,
				userId,
				toRequest(allLibraries, chosen, maxAge, blockUnrated)
			);
			saved = true;
			onchange?.(result.enforced && isRestricted(result));
		} catch (e) {
			error = accountErrorKey(e);
		} finally {
			busy = false;
		}
	}

	const nothingVisible = $derived(!allLibraries && chosen.length === 0);
</script>

<form onsubmit={save} class="mt-8 border-t border-white/10 pt-6" aria-labelledby="restrictions-title">
	<fieldset disabled={disabled || busy || isAdmin || !loaded} class="flex flex-col gap-4">
		<legend id="restrictions-title" class="mb-1 flex items-center gap-2 text-lg">
			<Icon name="lock" class="size-5 text-gold" />
			{t('restrictions.title')}
		</legend>
		{#if isAdmin}
			<p class="rounded-md bg-white/5 p-3 text-sm text-neutral-300">{t('restrictions.admin_note')}</p>
		{:else}
			<p class="text-sm text-neutral-400">{t('restrictions.hint')}</p>
		{/if}

		<div class="flex flex-wrap gap-2" role="group" aria-label={t('restrictions.presets')}>
			<button type="button" class="preset" onclick={() => preset('child')}>{t('restrictions.preset_child')}</button>
			<button type="button" class="preset" onclick={() => preset('teen')}>{t('restrictions.preset_teen')}</button>
			<button type="button" class="preset" onclick={() => preset('none')}>{t('restrictions.preset_none')}</button>
		</div>

		<div>
			<p class="mb-2 text-sm font-medium">{t('restrictions.libraries')}</p>
			<label class="check"
				><input type="radio" name="libraries-{userId}" checked={allLibraries} onchange={() => { allLibraries = true; saved = false; }} />{t('restrictions.all_libraries')}</label
			>
			<label class="check mt-1"
				><input type="radio" name="libraries-{userId}" checked={!allLibraries} onchange={() => { allLibraries = false; saved = false; }} />{t('restrictions.only_selected')}</label
			>
			{#if !allLibraries}
				<div class="mt-2 ml-7 flex flex-col gap-1.5">
					{#each libraries as library (library.id)}
						<label class="check">
							<input
								type="checkbox"
								checked={chosen.includes(library.id)}
								onchange={(e) => toggle(library.id, (e.currentTarget as HTMLInputElement).checked)}
							/>
							{library.name}
						</label>
					{/each}
					{#if nothingVisible}
						<p class="text-xs text-amber-300" role="status">{t('restrictions.nothing_visible')}</p>
					{/if}
				</div>
			{/if}
		</div>

		<label>
			{t('restrictions.age_limit')}
			<select
				value={maxAge === null ? '' : String(maxAge)}
				onchange={(e) => {
					const value = (e.currentTarget as HTMLSelectElement).value;
					maxAge = value === '' ? null : Number(value);
					saved = false;
				}}
			>
				<option value="">{t('restrictions.no_age_limit')}</option>
				{#each AGE_LIMITS as age (age)}
					<option value={String(age)}>{t(age === 0 ? 'restrictions.age_all' : 'restrictions.age_up_to', { age })}</option>
				{/each}
			</select>
		</label>
		<p class="-mt-2 text-xs text-neutral-500">{t('restrictions.age_examples')}</p>

		<label class="check {maxAge === null ? 'opacity-50' : ''}">
			<input type="checkbox" bind:checked={blockUnrated} disabled={maxAge === null} />
			{t('restrictions.block_unrated')}
		</label>

		{#if error}<p role="alert" class="text-sm text-red-300">{t(error)}</p>{/if}
		{#if saved}<p role="status" class="text-sm text-gold">{t('restrictions.saved')}</p>{/if}
		<div><button class="primary">{t('restrictions.save')}</button></div>
	</fieldset>
</form>

<style>
	button { border: 1px solid #525252; border-radius: 0.4rem; padding: 0.6rem 0.8rem; }
	.preset { padding: 0.35rem 0.7rem; font-size: 0.85rem; }
	.preset:hover { border-color: #d5ad42; }
	fieldset:disabled { opacity: 0.6; }
	.primary { background: #d5ad42; border-color: #d5ad42; color: #111; }
	label { font-size: 0.9rem; }
	select { display: block; width: 100%; margin-top: 0.4rem; border: 1px solid #525252; border-radius: 0.4rem; padding: 0.65rem; background: #171717; color: white; }
	.check { display: flex; align-items: center; gap: 0.65rem; }
	button:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid #d5ad42; outline-offset: 2px; }
</style>
