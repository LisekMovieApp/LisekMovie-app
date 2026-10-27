<!--
  Admin → Logs (Faza 17): the server's daily log files as a console. A
  day from the list, sources ([Wszystkie] [Backend] [Frontend] [FFmpeg]
  [FFprobe]) and levels (ERROR WARN INFO DEBUG) as toggles, a text filter,
  live refresh of today's file, and the window scrolls to the newest line
  unless the admin scrolled up to read.
-->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		LOG_LEVELS,
		LOG_SOURCES,
		clockTime,
		displayMessage,
		downloadLogFile,
		extraFields,
		fetchLogDay,
		fetchLogFiles,
		formatBytes,
		matchesText,
		type LogDay,
		type LogFiles,
		type LogLevel,
		type LogSource
	} from '$lib/admin/logs';

	const REFRESH_MS = 5000;

	let files = $state<LogFiles | null>(null);
	let date = $state('');
	let day = $state<LogDay | null>(null);
	let sources = $state<LogSource[]>([...LOG_SOURCES]);
	let levels = $state<LogLevel[]>(['error', 'warn', 'info']);
	let text = $state('');
	let live = $state(true);
	let loading = $state(false);
	let errorKey = $state<string | null>(null);
	let expanded = $state<Record<number, boolean>>({});
	let consoleEl = $state<HTMLElement>();
	/** The admin is at the bottom: new lines keep it there. */
	let pinned = true;

	const today = $derived(files?.files[0]?.date ?? '');
	const shown = $derived((day?.entries ?? []).filter((entry) => matchesText(entry, text)));

	const SOURCE_STYLE: Record<string, string> = {
		backend: 'text-sky-300',
		frontend: 'text-fuchsia-300',
		ffmpeg: 'text-emerald-300',
		ffprobe: 'text-teal-300'
	};
	const LEVEL_STYLE: Record<string, string> = {
		ERROR: 'bg-red-500/20 text-red-300',
		WARN: 'bg-amber-500/20 text-amber-200',
		INFO: 'bg-white/10 text-neutral-300',
		DEBUG: 'bg-white/5 text-neutral-500',
		TRACE: 'bg-white/5 text-neutral-600'
	};
	const SOURCE_LABEL: Record<LogSource, string> = {
		backend: 'Backend',
		frontend: 'Frontend',
		ffmpeg: 'FFmpeg',
		ffprobe: 'FFprobe'
	};

	async function loadFiles() {
		const token = getSessionCookieClientSide();
		if (!token) return;
		try {
			files = await fetchLogFiles(token);
			if (!date && files.files.length > 0) date = files.files[0].date;
		} catch {
			errorKey = 'logs.load_failed';
		}
	}

	async function loadDay(quiet = false) {
		const token = getSessionCookieClientSide();
		if (!token || !date) return;
		if (!quiet) loading = true;
		try {
			const next = await fetchLogDay(token, date, sources, levels);
			const grew = (day?.entries.length ?? 0) !== next.entries.length || day?.date !== next.date;
			day = next;
			errorKey = null;
			if (grew && pinned) {
				await tick();
				consoleEl?.scrollTo({ top: consoleEl.scrollHeight });
			}
		} catch {
			if (!quiet) errorKey = 'logs.load_failed';
		} finally {
			loading = false;
		}
	}

	function onScroll() {
		if (!consoleEl) return;
		pinned = consoleEl.scrollTop + consoleEl.clientHeight >= consoleEl.scrollHeight - 24;
	}

	function allSources() {
		sources = [...LOG_SOURCES];
	}

	function toggleSource(source: LogSource) {
		// From "all", a click shows only that source.
		if (sources.length === LOG_SOURCES.length) {
			sources = [source];
		} else if (sources.includes(source)) {
			sources = sources.length === 1 ? [...LOG_SOURCES] : sources.filter((s) => s !== source);
		} else {
			sources = [...sources, source];
		}
	}

	function toggleLevel(level: LogLevel) {
		levels = levels.includes(level)
			? levels.length === 1
				? levels
				: levels.filter((l) => l !== level)
			: LOG_LEVELS.filter((l) => l === level || levels.includes(l));
	}

	async function download() {
		const token = getSessionCookieClientSide();
		if (!token || !date) return;
		try {
			await downloadLogFile(token, date);
		} catch {
			errorKey = 'logs.download_failed';
		}
	}

	onMount(() => {
		void loadFiles();
	});

	// Day or filters changed: read again, from the bottom.
	$effect(() => {
		void date;
		void sources;
		void levels;
		pinned = true;
		expanded = {};
		void loadDay();
	});

	// Today's file grows: follow it.
	$effect(() => {
		if (!live || !date || date !== today) return;
		const timer = window.setInterval(() => void loadDay(true), REFRESH_MS);
		return () => window.clearInterval(timer);
	});

	const chip = (active: boolean) =>
		`rounded-md border px-2.5 py-1 text-xs font-medium transition-colors ${
			active
				? 'border-gold bg-gold/15 text-gold'
				: 'border-neutral-700 text-neutral-400 hover:border-neutral-500 hover:text-neutral-200'
		}`;
</script>

<main class="min-h-screen bg-base px-4 py-12 md:px-8">
	<div class="mx-auto max-w-6xl">
		<h1 class="text-2xl font-semibold text-neutral-100">{t('logs.title')}</h1>
		<p class="mt-1 text-sm text-neutral-400">
			{t('logs.subtitle', { days: files?.retention_days ?? 30 })}
		</p>

		<div class="mt-6 flex flex-wrap items-end gap-3">
			<label class="flex flex-col gap-1 text-sm text-neutral-300">
				{t('logs.day')}
				<select
					bind:value={date}
					class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold"
				>
					{#each files?.files ?? [] as file (file.date)}
						<option value={file.date}>
							{file.date === today ? `${file.date} · ${t('logs.today')}` : file.date} ({formatBytes(
								file.size_bytes
							)})
						</option>
					{/each}
				</select>
			</label>
			<label class="flex min-w-48 flex-1 flex-col gap-1 text-sm text-neutral-300">
				{t('logs.search')}
				<input
					bind:value={text}
					type="search"
					placeholder={t('logs.search_placeholder')}
					class="rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 placeholder:text-neutral-600 focus:border-gold"
				/>
			</label>
			<button
				type="button"
				onclick={() => loadDay()}
				disabled={loading || !date}
				class="rounded-md border border-neutral-700 px-3 py-2 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50"
			>
				{loading ? t('logs.loading') : t('logs.refresh')}
			</button>
			<button
				type="button"
				onclick={download}
				disabled={!date}
				class="rounded-md border border-neutral-700 px-3 py-2 text-sm text-neutral-200 hover:border-gold-dim disabled:opacity-50"
			>
				{t('logs.download')}
			</button>
		</div>

		<div class="mt-4 flex flex-wrap items-center gap-x-6 gap-y-3">
			<div class="flex flex-wrap items-center gap-2" role="group" aria-label={t('logs.sources')}>
				<button
					type="button"
					aria-pressed={sources.length === LOG_SOURCES.length}
					onclick={allSources}
					class={chip(sources.length === LOG_SOURCES.length)}
				>
					{t('logs.all')}
				</button>
				{#each LOG_SOURCES as source (source)}
					{@const active = sources.includes(source) && sources.length < LOG_SOURCES.length}
					<button
						type="button"
						aria-pressed={sources.includes(source)}
						onclick={() => toggleSource(source)}
						class={chip(active)}
					>
						{SOURCE_LABEL[source]}
					</button>
				{/each}
			</div>
			<div class="flex flex-wrap items-center gap-2" role="group" aria-label={t('logs.levels')}>
				{#each LOG_LEVELS as level (level)}
					<button
						type="button"
						aria-pressed={levels.includes(level)}
						onclick={() => toggleLevel(level)}
						class={chip(levels.includes(level))}
					>
						{level.toUpperCase()}
					</button>
				{/each}
			</div>
			{#if date && date === today}
				<label class="flex items-center gap-2 text-xs text-neutral-400">
					<input type="checkbox" bind:checked={live} class="size-4 accent-gold" />
					{t('logs.live')}
				</label>
			{/if}
		</div>

		{#if errorKey}
			<p role="alert" class="mt-4 rounded bg-red-950/40 p-3 text-sm text-red-200">{t(errorKey)}</p>
		{/if}

		<div
			bind:this={consoleEl}
			onscroll={onScroll}
			role="log"
			aria-live="off"
			aria-label={t('logs.console')}
			class="mt-4 h-[65vh] overflow-auto rounded-lg border border-white/10 bg-black/80 p-3 font-mono text-xs leading-relaxed"
		>
			{#if files && files.files.length === 0}
				<p class="text-neutral-500">{t('logs.no_files')}</p>
			{:else if day && shown.length === 0}
				<p class="text-neutral-500">{t('logs.no_entries')}</p>
			{:else}
				{#if day && (day.partial || day.total > day.entries.length)}
					<p class="mb-2 text-amber-300/80">
						{t('logs.cut', { shown: day.entries.length, total: day.total })}
					</p>
				{/if}
				{#each shown as entry, index (index)}
					{@const fields = extraFields(entry)}
					<div class="group border-b border-white/[0.03] py-0.5 hover:bg-white/[0.03]">
						<button
							type="button"
							disabled={fields.length === 0}
							onclick={() => (expanded = { ...expanded, [index]: !expanded[index] })}
							class="flex w-full flex-wrap items-baseline gap-x-2 text-left disabled:cursor-text sm:flex-nowrap"
						>
							<span class="shrink-0 text-neutral-500 tabular-nums"
								>{clockTime(entry.timestamp)}</span
							>
							<span
								class="w-12 shrink-0 rounded px-1 text-center text-[0.65rem] font-semibold {LEVEL_STYLE[
									entry.level
								] ?? LEVEL_STYLE.INFO}">{entry.level}</span
							>
							<span
								class="w-20 shrink-0 uppercase {SOURCE_STYLE[entry.source] ?? 'text-neutral-400'}"
								>[{entry.source}]</span
							>
							<span
								class="min-w-0 flex-1 basis-full break-words whitespace-pre-wrap sm:basis-auto {entry.level ===
								'ERROR'
									? 'text-red-200'
									: 'text-neutral-200'}">{displayMessage(entry)}</span
							>
							{#if fields.length > 0}
								<span class="shrink-0 text-neutral-600 group-hover:text-neutral-400"
									>{expanded[index] ? '▾' : '▸'}</span
								>
							{/if}
						</button>
						{#if expanded[index]}
							<dl
								class="mt-1 mb-1 grid grid-cols-[max-content_1fr] gap-x-3 text-neutral-400 sm:ml-24"
							>
								<dt class="text-neutral-600">{t('logs.target')}</dt>
								<dd class="break-all">{entry.target}</dd>
								{#each fields as [key, value] (key)}
									<dt class="text-neutral-600">{key}</dt>
									<dd class="break-all whitespace-pre-wrap">{value}</dd>
								{/each}
							</dl>
						{/if}
					</div>
				{/each}
			{/if}
		</div>
		{#if day}
			<p class="mt-2 text-xs text-neutral-500">
				{t('logs.count', { shown: shown.length, size: formatBytes(day.size_bytes) })}
			</p>
		{/if}
	</div>
</main>
