<!--
  Faza 26 — a channel's schedule: the slots the channel plays in order
  (what, in which order, how much, at what time, which ads around it), a
  search over the libraries to add one, a copy from another channel and a
  preview of the next hours. Saving replans the channel.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		ALL_DAYS,
		MAX_SLOTS,
		PLAY_ORDERS,
		SLOT_MODES,
		START_MODES,
		TAIL_MODES,
		WEEKDAYS,
		copySchedule,
		daysPreset,
		dayOn,
		entryLength,
		errorCode,
		fetchChannels,
		fetchPresets,
		fetchSlots,
		formatLength,
		moveItem,
		newSlot,
		normalizeMode,
		previewChannel,
		rebuildChannel,
		saveSlots,
		searchSources,
		slotProblem,
		toInput,
		toggleDay,
		type AdminChannel,
		type FillerPreset,
		type PlayoutEntry,
		type SlotInput,
		type SourceHit
	} from '$lib/tv/api';

	type Row = { key: number; slot: SlotInput; label: string };

	const channelId = $derived(page.params.id ?? '');
	let token = '';
	let channel = $state<AdminChannel | null>(null);
	let others = $state<AdminChannel[]>([]);
	let presets = $state<FillerPreset[]>([]);
	let rows = $state<Row[]>([]);
	let saved = $state('[]');
	let loading = $state(true);
	let loadFailed = $state(false);
	let errorKey = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);
	let query = $state('');
	let hits = $state<SourceHit[]>([]);
	let searching = $state(false);
	let copyFrom = $state('');
	let preview = $state<PlayoutEntry[]>([]);
	let warnings = $state<string[]>([]);
	let confirming = $state<'reset' | 'copy' | null>(null);
	let counter = 0;
	let searchTimer: ReturnType<typeof setTimeout> | undefined;

	const snapshot = (list: Row[]) => JSON.stringify(list.map((r) => r.slot));
	const dirty = $derived(snapshot(rows) !== saved);
	const problem = $derived(rows.map((r) => slotProblem(r.slot)).find((p) => p !== null) ?? null);
	const presetsOf = (kind: string) => presets.filter((p) => p.kind === kind);

	function load() {
		loading = true;
		Promise.all([fetchChannels(token), fetchSlots(token, channelId), fetchPresets(token)])
			.then(([list, slots, filler]) => {
				channel = list.find((c) => c.id === channelId) ?? null;
				others = list.filter((c) => c.id !== channelId && c.slot_count > 0);
				presets = filler;
				rows = slots.map((s) => ({ key: ++counter, slot: toInput(s), label: s.source_label ?? '' }));
				saved = snapshot(rows);
				loadFailed = channel === null;
				if (channel) void refreshPreview();
			})
			.catch(() => (loadFailed = true))
			.finally(() => (loading = false));
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		load();
		return () => clearTimeout(searchTimer);
	});

	async function refreshPreview() {
		try {
			preview = await previewChannel(token, channelId, 12);
		} catch {
			preview = [];
		}
	}

	function onSearch() {
		clearTimeout(searchTimer);
		if (query.trim().length < 2) {
			hits = [];
			return;
		}
		searchTimer = setTimeout(async () => {
			searching = true;
			try {
				hits = await searchSources(token, query.trim());
			} catch {
				hits = [];
			} finally {
				searching = false;
			}
		}, 250);
	}

	function add(hit: SourceHit) {
		if (rows.length >= MAX_SLOTS) {
			errorKey = 'tv_admin.errors.too_many_slots';
			return;
		}
		rows = [...rows, { key: ++counter, slot: newSlot(hit), label: hit.title }];
		query = '';
		hits = [];
	}

	function patch(index: number, change: Partial<SlotInput>) {
		rows = rows.map((r, i) => (i === index ? { ...r, slot: normalizeMode({ ...r.slot, ...change }) } : r));
	}

	function setDays(index: number, mask: number) {
		patch(index, { days_of_week: mask });
	}

	async function save() {
		if (problem || busy) return;
		busy = true;
		errorKey = null;
		notice = null;
		try {
			const result = await saveSlots(
				token,
				channelId,
				rows.map((r) => r.slot)
			);
			rows = result.map((s) => ({ key: ++counter, slot: toInput(s), label: s.source_label ?? '' }));
			saved = snapshot(rows);
			notice = 'tv_admin.saved_replanning';
			await rebuild('refresh', false);
		} catch (error) {
			const code = errorCode(error);
			errorKey = code === 'invalid_slot' ? 'tv_admin.errors.invalid_slot' : 'tv_admin.errors.save_failed';
		} finally {
			busy = false;
		}
	}

	async function rebuild(mode: 'continue' | 'refresh' | 'reset', manual = true) {
		if (manual) busy = true;
		confirming = null;
		try {
			const result = await rebuildChannel(token, channelId, mode);
			warnings = result.warnings;
			await refreshPreview();
			if (manual) notice = 'tv_admin.rebuilt';
		} catch {
			errorKey = 'tv_admin.errors.rebuild_failed';
		} finally {
			if (manual) busy = false;
		}
	}

	async function copy() {
		if (!copyFrom) return;
		if (confirming !== 'copy' && rows.length > 0) {
			confirming = 'copy';
			return;
		}
		confirming = null;
		busy = true;
		errorKey = null;
		try {
			const slots = await copySchedule(token, channelId, copyFrom);
			rows = slots.map((s) => ({ key: ++counter, slot: toInput(s), label: s.source_label ?? '' }));
			saved = snapshot(rows);
			await refreshPreview();
			notice = 'tv_admin.copied';
		} catch {
			errorKey = 'tv_admin.errors.copy_failed';
		} finally {
			busy = false;
		}
	}

	function clock(iso: string) {
		return new Intl.DateTimeFormat(getLocale(), { hour: '2-digit', minute: '2-digit' }).format(new Date(iso));
	}

	const field =
		'rounded-md border border-neutral-700 bg-black px-2 py-1.5 text-sm text-neutral-100 focus:border-gold';
	const button =
		'rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:border-gold-dim disabled:opacity-50';
	const kindBadge: Record<string, string> = {
		content: 'bg-gold/20 text-gold',
		pre: 'bg-sky-500/20 text-sky-300',
		mid: 'bg-sky-500/20 text-sky-300',
		post: 'bg-sky-500/20 text-sky-300',
		tail: 'bg-sky-500/20 text-sky-300',
		pad: 'bg-white/10 text-neutral-300'
	};
</script>

<svelte:head><title>{channel?.name ?? t('tv_admin.schedule')} · LisekMovie</title></svelte:head>

<main class="min-h-screen bg-base px-4 py-8 md:px-8">
	<div class="mx-auto max-w-5xl">
		<a href={resolve('/admin/tv')} class="text-sm text-neutral-400 hover:text-gold">
			← {t('tv_admin.back_to_channels')}
		</a>
		{#if loading}
			<p class="mt-4 text-sm text-neutral-400">{t('admin.loading')}</p>
		{:else if loadFailed || !channel}
			<p class="mt-4 text-sm text-red-400">{t('admin.load_failed')}</p>
		{:else}
			<h1 class="mt-2 text-2xl font-semibold text-neutral-100">
				{t('tv_admin.schedule_of', { number: channel.number, name: channel.name })}
			</h1>
			<p class="mt-1 text-sm text-neutral-400">{t('tv_admin.schedule_hint', { zone: channel.timezone })}</p>

			{#if errorKey}<p role="alert" class="mt-3 text-sm text-red-400">{t(errorKey)}</p>{/if}
			{#if notice}<p role="status" class="mt-3 text-sm text-green-400">{t(notice)}</p>{/if}

			<section class="mt-6 rounded-lg border border-white/10 bg-surface/60 p-4">
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('tv_admin.add_source')}
					<input
						class="{field} px-3 py-2"
						type="search"
						bind:value={query}
						oninput={onSearch}
						placeholder={t('tv_admin.search_placeholder')}
					/>
				</label>
				{#if searching}
					<p class="mt-2 text-xs text-neutral-500">{t('tv_admin.searching')}</p>
				{:else if hits.length > 0}
					<ul class="mt-2 max-h-64 overflow-y-auto rounded-md ring-1 ring-white/10">
						{#each hits as hit (hit.kind + hit.id)}
							<li>
								<button
									type="button"
									class="flex w-full items-center gap-3 px-3 py-2 text-left text-sm hover:bg-white/5"
									onclick={() => add(hit)}
								>
									<span class="rounded bg-white/10 px-2 py-0.5 text-xs text-neutral-300">
										{t(`tv_admin.kind.${hit.kind}`)}
									</span>
									<span class="min-w-0 flex-1 truncate text-neutral-100">{hit.title}</span>
									{#if hit.detail}<span class="text-xs text-neutral-500">{hit.detail}</span>{/if}
								</button>
							</li>
						{/each}
					</ul>
				{:else if query.trim().length >= 2}
					<p class="mt-2 text-xs text-neutral-500">{t('tv_admin.no_hits')}</p>
				{/if}
			</section>

			<section class="mt-6" aria-label={t('tv_admin.slots')}>
				{#if rows.length === 0}
					<p class="text-sm text-neutral-400">{t('tv_admin.no_slots')}</p>
				{/if}
				<ol class="flex flex-col gap-3">
					{#each rows as row, index (row.key)}
						{@const slot = row.slot}
						<li class="rounded-lg border border-white/10 bg-surface/60 p-4">
							<div class="flex flex-wrap items-center gap-2">
								<span class="text-sm font-semibold text-gold">{index + 1}.</span>
								<span class="rounded bg-white/10 px-2 py-0.5 text-xs text-neutral-300">
									{t(`tv_admin.kind.${slot.source_kind}`)}
								</span>
								<span class="min-w-0 flex-1 truncate font-medium text-neutral-100">{row.label}</span>
								<button type="button" class={button} disabled={index === 0} aria-label={t('tv_admin.move_up')}
									onclick={() => (rows = moveItem(rows, index, index - 1))}>↑</button>
								<button type="button" class={button} disabled={index === rows.length - 1} aria-label={t('tv_admin.move_down')}
									onclick={() => (rows = moveItem(rows, index, index + 1))}>↓</button>
								<button type="button" class={button} onclick={() => (rows = rows.filter((_, i) => i !== index))}>
									{t('tv_admin.remove_slot')}
								</button>
							</div>

							<div class="mt-3 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
								<label class="flex flex-col gap-1 text-xs text-neutral-400">
									{t('tv_admin.slot_time')}
									<input
										type="time"
										class={field}
										value={slot.start_time ?? ''}
										onchange={(e) => patch(index, { start_time: e.currentTarget.value || null })}
									/>
								</label>
								{#if slot.start_time}
									<label class="flex flex-col gap-1 text-xs text-neutral-400">
										{t('tv_admin.slot_start_mode')}
										<select class={field} value={slot.start_mode} onchange={(e) => patch(index, { start_mode: e.currentTarget.value })}>
											{#each START_MODES as m (m)}<option value={m}>{t(`tv_admin.start_mode.${m}`)}</option>{/each}
										</select>
									</label>
								{/if}
								<label class="flex flex-col gap-1 text-xs text-neutral-400">
									{t('tv_admin.slot_order')}
									<select class={field} value={slot.play_order} onchange={(e) => patch(index, { play_order: e.currentTarget.value })}>
										{#each PLAY_ORDERS as o (o)}<option value={o}>{t(`tv_admin.order.${o}`)}</option>{/each}
									</select>
								</label>
								<label class="flex flex-col gap-1 text-xs text-neutral-400">
									{t('tv_admin.slot_mode')}
									<select class={field} value={slot.mode} onchange={(e) => patch(index, { mode: e.currentTarget.value })}>
										{#each SLOT_MODES as m (m)}<option value={m}>{t(`tv_admin.mode.${m}`)}</option>{/each}
									</select>
								</label>
								{#if slot.mode === 'multiple'}
									<label class="flex flex-col gap-1 text-xs text-neutral-400">
										{t('tv_admin.slot_count')}
										<input type="number" min="1" max="100" class={field} value={slot.count ?? ''}
											onchange={(e) => patch(index, { count: Number(e.currentTarget.value) || null })} />
									</label>
								{:else if slot.mode === 'duration'}
									<label class="flex flex-col gap-1 text-xs text-neutral-400">
										{t('tv_admin.slot_duration')}
										<input type="number" min="1" max="1440" class={field} value={slot.duration_minutes ?? ''}
											onchange={(e) => patch(index, { duration_minutes: Number(e.currentTarget.value) || null })} />
									</label>
									<label class="flex flex-col gap-1 text-xs text-neutral-400">
										{t('tv_admin.slot_tail')}
										<select class={field} value={slot.tail_mode} onchange={(e) => patch(index, { tail_mode: e.currentTarget.value })}>
											{#each TAIL_MODES as m (m)}<option value={m}>{t(`tv_admin.tail.${m}`)}</option>{/each}
										</select>
									</label>
								{/if}
							</div>

							<div class="mt-3 flex flex-wrap items-center gap-1.5" role="group" aria-label={t('tv_admin.slot_days')}>
								{#each WEEKDAYS as day, d (day)}
									<button
										type="button"
										aria-pressed={dayOn(slot.days_of_week, d)}
										class="rounded px-2.5 py-1 text-xs {dayOn(slot.days_of_week, d) ? 'bg-gold text-black' : 'bg-white/5 text-neutral-400'}"
										onclick={() => setDays(index, toggleDay(slot.days_of_week, d))}
									>
										{t(`tv_admin.day.${day}`)}
									</button>
								{/each}
								<button type="button" class="ml-2 text-xs text-neutral-400 hover:text-gold" onclick={() => setDays(index, ALL_DAYS)}>
									{t('tv_admin.days_all')}
								</button>
								<button type="button" class="text-xs text-neutral-400 hover:text-gold" onclick={() => setDays(index, 0b0011111)}>
									{t('tv_admin.days_weekdays')}
								</button>
								<button type="button" class="text-xs text-neutral-400 hover:text-gold" onclick={() => setDays(index, 0b1100000)}>
									{t('tv_admin.days_weekend')}
								</button>
								{#if daysPreset(slot.days_of_week) === null && slot.days_of_week === 0}
									<span class="text-xs text-red-400">{t('tv_admin.problem_days')}</span>
								{/if}
							</div>

							<div class="mt-3 grid gap-3 sm:grid-cols-3">
								{#each [['pre_filler_id', 'pre'], ['post_filler_id', 'post'], ['tail_filler_id', 'tail']] as [fieldName, kind] (fieldName)}
									{@const key = fieldName as 'pre_filler_id' | 'post_filler_id' | 'tail_filler_id'}
									<label class="flex flex-col gap-1 text-xs text-neutral-400">
										{t(`tv_admin.filler_${kind}`)}
										<select class={field} value={slot[key] ?? ''}
											onchange={(e) => patch(index, { [key]: e.currentTarget.value || null })}>
											<option value="">{t('tv_admin.filler_none')}</option>
											{#each presetsOf(kind) as preset (preset.id)}
												<option value={preset.id}>{preset.name}</option>
											{/each}
										</select>
									</label>
								{/each}
							</div>

							<div class="mt-3 grid gap-3 sm:grid-cols-2">
								<label class="flex flex-col gap-1 text-xs text-neutral-400">
									{t('tv_admin.slot_title_override')}
									<input class={field} maxlength="200" value={slot.title_override ?? ''}
										onchange={(e) => patch(index, { title_override: e.currentTarget.value.trim() || null })} />
								</label>
								<label class="flex items-center gap-2 pt-5 text-sm text-neutral-300">
									<input type="checkbox" checked={slot.in_guide} onchange={(e) => patch(index, { in_guide: e.currentTarget.checked })} />
									{t('tv_admin.slot_in_guide')}
								</label>
							</div>
							{#if slotProblem(slot)}
								<p class="mt-2 text-xs text-red-400">{t(slotProblem(slot) ?? '')}</p>
							{/if}
						</li>
					{/each}
				</ol>
			</section>

			<div class="sticky bottom-0 mt-6 flex flex-wrap items-center gap-2 border-t border-white/10 bg-base/95 py-3 backdrop-blur">
				<button type="button" disabled={!dirty || !!problem || busy}
					class="rounded-md bg-gold px-4 py-2 text-sm font-medium text-black hover:bg-gold-dim disabled:opacity-50"
					onclick={save}>
					{busy ? t('tv_admin.saving') : t('tv_admin.save_schedule')}
				</button>
				<button type="button" class={button} disabled={!dirty || busy} onclick={load}>{t('tv_admin.discard')}</button>
				{#if dirty}<span class="text-xs text-amber-300">{t('tv_admin.unsaved')}</span>{/if}
				<span class="grow"></span>
				{#if others.length > 0}
					<select class={field} bind:value={copyFrom} aria-label={t('tv_admin.copy_from')}>
						<option value="">{t('tv_admin.copy_from')}</option>
						{#each others as other (other.id)}<option value={other.id}>{other.number} {other.name}</option>{/each}
					</select>
					<button type="button" class={button} disabled={!copyFrom || busy} onclick={copy}>
						{confirming === 'copy' ? t('tv_admin.confirm_copy') : t('tv_admin.copy')}
					</button>
				{/if}
				<button type="button" class={button} disabled={busy}
					onclick={() => (confirming === 'reset' ? rebuild('reset') : (confirming = 'reset'))}>
					{confirming === 'reset' ? t('tv_admin.confirm_reset') : t('tv_admin.rebuild_reset')}
				</button>
			</div>

			<section class="mt-8" aria-label={t('tv_admin.preview')}>
				<h2 class="text-lg font-semibold text-neutral-100">{t('tv_admin.preview')}</h2>
				<p class="text-xs text-neutral-500">{t('tv_admin.preview_hint')}</p>
				{#each warnings as warning (warning)}
					<p class="mt-2 text-xs text-amber-400">{warning}</p>
				{/each}
				{#if preview.length === 0}
					<p class="mt-2 text-sm text-neutral-400">{t('tv_admin.preview_empty')}</p>
				{:else}
					<div class="mt-2 overflow-x-auto rounded-lg ring-1 ring-white/10">
						<table class="w-full text-left text-sm">
							<tbody>
								{#each preview as entry (entry.starts_at)}
									<tr class="border-t border-white/5 first:border-0">
										<td class="p-2 whitespace-nowrap text-neutral-400">{clock(entry.starts_at)}</td>
										<td class="p-2">
											<span class="rounded px-2 py-0.5 text-xs {kindBadge[entry.kind] ?? ''}">
												{t(`tv_admin.entry.${entry.kind}`)}
											</span>
										</td>
										<td class="min-w-0 p-2 text-neutral-100">
											{entry.title}
											{#if entry.subtitle}<span class="text-neutral-500"> · {entry.subtitle}</span>{/if}
										</td>
										<td class="p-2 text-right whitespace-nowrap text-neutral-400">{formatLength(entryLength(entry))}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
				{/if}
			</section>
		{/if}
	</div>
</main>
