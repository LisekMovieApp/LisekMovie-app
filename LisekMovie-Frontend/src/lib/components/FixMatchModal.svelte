<!--
  "Napraw dopasowanie" (Fix match, Faza 7). When the file name fooled the
  scanner, an administrator searches TMDB/TheTVDB by hand (title, year,
  provider), picks the right entry and confirms. The server renames the
  film — or every episode of the series — to the provider's title, swaps
  the artwork and locks the item, so later scans keep the fix.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { navScope } from '$lib/spatial/spatial';
	import { ApiError } from '$lib/playback/api';
	import {
		identifyItem,
		parseYear,
		searchProviders,
		type IdentifyResult,
		type MatchKind,
		type ProviderCandidate
	} from '$lib/media/identify';

	interface Props {
		itemId: string;
		kind: MatchKind;
		title: string;
		year: number | null;
		onClose: () => void;
		onIdentified: (result: IdentifyResult) => void;
	}

	let { itemId, kind, title, year, onClose, onIdentified }: Props = $props();

	// svelte-ignore state_referenced_locally
	let query = $state(title);
	// svelte-ignore state_referenced_locally
	let yearText = $state(year !== null ? String(year) : '');
	let provider = $state('');
	let results = $state<ProviderCandidate[] | null>(null);
	let failures = $state<string[]>([]);
	let searching = $state(false);
	let applying = $state(false);
	let errorKey = $state<string | null>(null);
	let picked = $state<ProviderCandidate | null>(null);
	let failedPosters = $state<Record<string, boolean>>({});

	/** Provider brand names are not translated. */
	const PROVIDERS = [
		{ key: 'tmdb', name: 'TMDB' },
		{ key: 'tvdb', name: 'TheTVDB' }
	] as const;

	const KNOWN_ERRORS = new Set([
		'provider_not_configured',
		'provider_item_not_found',
		'provider_failed',
		'invalid_query'
	]);

	function errorFrom(err: unknown): string {
		const code = err instanceof ApiError ? err.code : null;
		return `fix_match.errors.${code && KNOWN_ERRORS.has(code) ? code : 'generic'}`;
	}

	async function search(event?: SubmitEvent) {
		event?.preventDefault();
		const token = getSessionCookieClientSide();
		if (!token || !query.trim() || searching) return;
		searching = true;
		errorKey = null;
		picked = null;
		try {
			const found = await searchProviders(token, kind, query, parseYear(yearText), provider || null);
			results = found.results;
			failures = found.failures.map((f) => f.provider);
		} catch (err) {
			errorKey = errorFrom(err);
			results = null;
		} finally {
			searching = false;
		}
	}

	async function apply() {
		const token = getSessionCookieClientSide();
		if (!token || !picked || applying) return;
		applying = true;
		errorKey = null;
		try {
			onIdentified(await identifyItem(token, itemId, picked.provider, picked.provider_id));
		} catch (err) {
			errorKey = errorFrom(err);
		} finally {
			applying = false;
		}
	}

	function keydown(event: KeyboardEvent) {
		if (event.key === 'Escape' && !applying) onClose();
	}

	$effect(() => {
		void search();
	});

	const key = (c: ProviderCandidate) => `${c.provider}:${c.provider_id}`;
</script>

<svelte:window onkeydown={keydown} />

<div
	role="presentation"
	class="fixed inset-0 z-50 flex items-center justify-center bg-black/75 p-4"
	onclick={(e) => e.target === e.currentTarget && !applying && onClose()}
>
	<div
		role="dialog"
		aria-modal="true"
		aria-labelledby="fix-match-title"
		class="flex max-h-[90vh] w-full max-w-2xl flex-col rounded-lg border border-white/10 bg-surface text-neutral-100"
		use:navScope={{ autofocus: true }}
	>
		<div class="border-b border-white/10 p-5">
			<h2 id="fix-match-title" class="text-lg font-semibold">{t('fix_match.title')}</h2>
			<p class="mt-1 text-sm text-neutral-400">
				{t(kind === 'series' ? 'fix_match.hint_series' : 'fix_match.hint_movie')}
			</p>
			<form class="mt-4 flex flex-wrap items-end gap-3" onsubmit={search}>
				<label class="flex min-w-48 flex-1 flex-col gap-1 text-sm text-neutral-300">
					{t('fix_match.query')}
					<input
						bind:value={query}
						required
						maxlength="200"
						class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					/>
				</label>
				<label class="flex w-24 flex-col gap-1 text-sm text-neutral-300">
					{t('fix_match.year')}
					<input
						bind:value={yearText}
						inputmode="numeric"
						maxlength="4"
						class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					/>
				</label>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('fix_match.provider')}
					<select
						bind:value={provider}
						class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
					>
						<option value="">{t('fix_match.all_providers')}</option>
						{#each PROVIDERS as entry (entry.key)}
							<option value={entry.key}>{entry.name}</option>
						{/each}
					</select>
				</label>
				<button
					type="submit"
					disabled={searching}
					class="rounded-md bg-gold px-4 py-2 font-medium text-black hover:bg-gold-dim disabled:opacity-50"
				>
					{searching ? t('fix_match.searching') : t('fix_match.search')}
				</button>
			</form>
		</div>

		<div class="min-h-40 flex-1 overflow-y-auto p-5">
			{#if errorKey}
				<p role="alert" class="mb-3 rounded bg-red-950/40 p-3 text-sm text-red-200">{t(errorKey)}</p>
			{/if}
			{#if failures.length > 0}
				<p class="mb-3 text-xs text-amber-300">
					{t('fix_match.provider_failed_some', { providers: failures.join(', ') })}
				</p>
			{/if}
			{#if results === null && searching}
				<p class="text-sm text-neutral-400">{t('fix_match.searching')}</p>
			{:else if results !== null && results.length === 0}
				<p class="text-sm text-neutral-400">{t('fix_match.no_results')}</p>
			{:else if results}
				<ul class="flex flex-col gap-2" role="listbox" aria-label={t('fix_match.results')}>
					{#each results as candidate (key(candidate))}
						{@const selected = picked !== null && key(picked) === key(candidate)}
						<li>
							<button
								type="button"
								role="option"
								aria-selected={selected}
								onclick={() => (picked = candidate)}
								class="flex w-full gap-3 rounded-md border p-2 text-left transition-colors {selected
									? 'border-gold bg-gold/10'
									: 'border-white/10 hover:border-white/30'}"
							>
								<div class="h-24 w-16 shrink-0 overflow-hidden rounded bg-black">
									{#if candidate.poster_url && !failedPosters[key(candidate)]}
										<img
											src={candidate.poster_url}
											alt=""
											loading="lazy"
											referrerpolicy="no-referrer"
											onerror={() => (failedPosters = { ...failedPosters, [key(candidate)]: true })}
											class="size-full object-cover"
										/>
									{/if}
								</div>
								<div class="min-w-0 flex-1">
									<p class="font-medium">
										{candidate.title}
										{#if candidate.year}<span class="text-neutral-400">{` (${candidate.year})`}</span>{/if}
									</p>
									<p class="text-xs text-gold">
										{`${candidate.provider.toUpperCase()} · ${candidate.provider_id}`}
									</p>
									{#if candidate.overview}
										<p class="mt-1 line-clamp-2 text-xs text-neutral-400">{candidate.overview}</p>
									{/if}
								</div>
							</button>
						</li>
					{/each}
				</ul>
			{/if}
		</div>

		<div class="flex flex-wrap items-center justify-end gap-3 border-t border-white/10 p-4">
			{#if picked}
				<p class="mr-auto text-sm text-neutral-300">
					{t(kind === 'series' ? 'fix_match.confirm_series' : 'fix_match.confirm_movie', {
						title: picked.title
					})}
				</p>
			{/if}
			<button
				type="button"
				disabled={applying}
				onclick={onClose}
				class="rounded-md border border-neutral-700 px-4 py-2 text-sm hover:border-gold-dim disabled:opacity-50"
			>
				{t('fix_match.cancel')}
			</button>
			<button
				type="button"
				disabled={!picked || applying}
				onclick={apply}
				class="rounded-md bg-gold px-4 py-2 text-sm font-medium text-black hover:bg-gold-dim disabled:opacity-40"
			>
				{applying ? t('fix_match.applying') : t('fix_match.apply')}
			</button>
		</div>
	</div>
</div>
