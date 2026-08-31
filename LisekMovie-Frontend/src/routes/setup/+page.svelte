<!--
  First-Run Wizard: language → database connection → admin account.
  Single-page, three in-memory steps (not three routes) so nothing is lost
  between them and the URL stays just "/setup". `+layout.ts` is what sends a
  browser here in the first place, and away from here once setup is done.
-->
<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t, setLocale, getLocale, setLocaleCookie, type Locale } from '$lib/i18n';
	import logoText from '$lib/assets/logo_text.svg';

	interface DatabaseConfig {
		host: string;
		port: number;
		database: string;
		username: string;
		password: string;
	}

	interface SetupErrorBody {
		error_code: string;
		message: string;
	}

	// Matches this project's docker-compose.yml exactly — the "use the
	// bundled Postgres" path is only convenient if these stay in sync with
	// that file.
	const COMPOSE_DEFAULTS: DatabaseConfig = {
		host: 'localhost',
		port: 5432,
		database: 'lisekmovie',
		username: 'lisekmovie',
		password: 'lisekmovie'
	};

	const BLANK_DATABASE: DatabaseConfig = {
		host: '',
		port: 5432,
		database: '',
		username: '',
		password: ''
	};

	const TOTAL_STEPS = 3;
	const inputClass =
		'rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold';

	let step = $state(1);
	let uiLanguage = $state<Locale>(getLocale());

	let databaseSource = $state<'compose' | 'custom'>('compose');
	let database = $state<DatabaseConfig>({ ...COMPOSE_DEFAULTS });

	let testState = $state<'idle' | 'testing' | 'ok' | 'error'>('idle');
	let testErrorCode = $state<string | null>(null);

	let adminUsername = $state('');
	let adminPassword = $state('');
	let confirmPassword = $state('');
	let finishing = $state(false);
	let finishErrorCode = $state<string | null>(null);

	let passwordsMatch = $derived(adminPassword.length > 0 && adminPassword === confirmPassword);
	let adminStepValid = $derived(
		adminUsername.trim().length > 0 && adminPassword.length >= 8 && passwordsMatch
	);

	function selectLanguage(locale: Locale) {
		uiLanguage = locale;
		setLocale(locale);
		// Persisted immediately, not just held at `finishSetup` time — this is
		// the only record of the choice that survives a full page load, and
		// it's what lets the *next* page (the login screen `finishSetup`
		// redirects to, since finishing setup doesn't sign anyone in) render
		// in the right language too, not just the rest of this wizard.
		setLocaleCookie(locale);
	}

	function selectDatabaseSource(source: 'compose' | 'custom') {
		databaseSource = source;
		database = { ...(source === 'compose' ? COMPOSE_DEFAULTS : BLANK_DATABASE) };
		testState = 'idle';
		testErrorCode = null;
	}

	// A previously successful "test connection" no longer proves anything
	// once a field has changed since — require re-testing before Continue
	// re-enables.
	function invalidatePreviousTest() {
		if (testState !== 'idle') {
			testState = 'idle';
			testErrorCode = null;
		}
	}

	async function postJson<T>(
		url: string,
		body: unknown
	): Promise<{ ok: true; data: T } | { ok: false; errorCode: string }> {
		try {
			const response = await fetch(url, {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify(body)
			});
			if (!response.ok) {
				const errorBody = (await response.json().catch(() => null)) as SetupErrorBody | null;
				return { ok: false, errorCode: errorBody?.error_code ?? 'internal_error' };
			}
			return { ok: true, data: (await response.json()) as T };
		} catch {
			return { ok: false, errorCode: 'network_error' };
		}
	}

	async function testConnection() {
		testState = 'testing';
		testErrorCode = null;
		const result = await postJson<{ ok: boolean }>('/api/setup/database/test', database);
		if (result.ok) {
			testState = 'ok';
		} else {
			testState = 'error';
			testErrorCode = result.errorCode;
		}
	}

	async function finishSetup() {
		if (!adminStepValid) return;
		finishing = true;
		finishErrorCode = null;

		const result = await postJson<{ ok: boolean }>('/api/setup/finish', {
			database,
			admin_username: adminUsername,
			admin_password: adminPassword,
			ui_language: uiLanguage
		});

		finishing = false;

		if (result.ok) {
			// `invalidateAll` matters here, not just style: without it this is a
			// plain client-side navigation, and SvelteKit reuses the root
			// layout's already-cached load result rather than re-running
			// `+layout.server.ts` — so it would still be reading the request
			// that rendered /setup, before the language cookie
			// `selectLanguage` just set even existed. That cached result is
			// also what sends this browser on to /login in the first place
			// (setup finishing doesn't sign anyone in) — the same page that
			// needs the fresh cookie to render in the chosen language at all.
			await goto(resolve('/'), { invalidateAll: true });
		} else {
			finishErrorCode = result.errorCode;
		}
	}
</script>

<main class="flex min-h-screen flex-col items-center justify-center bg-base px-6 py-12">
	<div class="w-full max-w-md">
		<div class="mb-8 flex flex-col items-center gap-2 text-center">
			<img src={logoText} alt="LisekMovie" class="w-40" />
			<p class="text-sm tracking-wide text-gold-dim uppercase">
				{t('setup.step_indicator', { current: step, total: TOTAL_STEPS })}
			</p>
		</div>

		<div class="rounded-lg border border-surface bg-surface/60 p-8">
			{#if step === 1}
				<h1 class="mb-1 text-xl font-semibold text-neutral-50">
					{t('setup.step_language_title')}
				</h1>
				<p class="mb-6 text-sm text-neutral-400">{t('setup.step_language_subtitle')}</p>

				<div class="flex flex-col gap-3">
					<button
						type="button"
						class="rounded-md border px-4 py-3 text-left transition-colors {uiLanguage === 'pl'
							? 'border-gold bg-gold/10 text-gold'
							: 'border-neutral-700 text-neutral-200 hover:border-gold-dim'}"
						onclick={() => selectLanguage('pl')}
					>
						Polski
					</button>
					<button
						type="button"
						class="rounded-md border px-4 py-3 text-left transition-colors {uiLanguage === 'en'
							? 'border-gold bg-gold/10 text-gold'
							: 'border-neutral-700 text-neutral-200 hover:border-gold-dim'}"
						onclick={() => selectLanguage('en')}
					>
						English
					</button>
				</div>

				<button
					type="button"
					class="mt-8 w-full rounded-md bg-gold px-4 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim"
					onclick={() => (step = 2)}
				>
					{t('common.next')}
				</button>
			{:else if step === 2}
				<h1 class="mb-1 text-xl font-semibold text-neutral-50">
					{t('setup.step_database_title')}
				</h1>
				<p class="mb-6 text-sm text-neutral-400">{t('setup.step_database_subtitle')}</p>

				<div class="mb-6 flex flex-col gap-3">
					<button
						type="button"
						class="rounded-md border px-4 py-3 text-left transition-colors {databaseSource ===
						'compose'
							? 'border-gold bg-gold/10'
							: 'border-neutral-700 hover:border-gold-dim'}"
						onclick={() => selectDatabaseSource('compose')}
					>
						<span class="block text-sm font-medium text-neutral-100"
							>{t('setup.source_compose')}</span
						>
						<span class="block text-xs text-neutral-400">{t('setup.source_compose_hint')}</span>
					</button>
					<button
						type="button"
						class="rounded-md border px-4 py-3 text-left transition-colors {databaseSource ===
						'custom'
							? 'border-gold bg-gold/10'
							: 'border-neutral-700 hover:border-gold-dim'}"
						onclick={() => selectDatabaseSource('custom')}
					>
						<span class="block text-sm font-medium text-neutral-100"
							>{t('setup.source_custom')}</span
						>
						<span class="block text-xs text-neutral-400">{t('setup.source_custom_hint')}</span>
					</button>
				</div>

				<div class="flex flex-col gap-3">
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_host')}
						<input
							type="text"
							class={inputClass}
							bind:value={database.host}
							oninput={invalidatePreviousTest}
						/>
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_port')}
						<input
							type="number"
							class={inputClass}
							bind:value={database.port}
							oninput={invalidatePreviousTest}
						/>
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_database')}
						<input
							type="text"
							class={inputClass}
							bind:value={database.database}
							oninput={invalidatePreviousTest}
						/>
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_username')}
						<input
							type="text"
							class={inputClass}
							bind:value={database.username}
							oninput={invalidatePreviousTest}
						/>
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_password')}
						<input
							type="password"
							class={inputClass}
							bind:value={database.password}
							oninput={invalidatePreviousTest}
						/>
					</label>
				</div>

				{#if testState === 'error' && testErrorCode}
					<p class="mt-4 text-sm text-red-400">{t(`errors.${testErrorCode}`)}</p>
				{:else if testState === 'ok'}
					<p class="mt-4 text-sm text-emerald-400">{t('setup.test_connection_ok')}</p>
				{/if}

				<button
					type="button"
					class="mt-4 w-full rounded-md border border-gold-dim px-4 py-2.5 font-medium text-gold transition-colors hover:bg-gold/10 disabled:cursor-not-allowed disabled:opacity-50"
					disabled={testState === 'testing'}
					onclick={testConnection}
				>
					{testState === 'testing' ? t('setup.testing') : t('setup.test_connection')}
				</button>

				<div class="mt-4 flex gap-3">
					<button
						type="button"
						class="w-1/3 rounded-md border border-neutral-700 px-4 py-2.5 font-medium text-neutral-200 hover:border-gold-dim"
						onclick={() => (step = 1)}
					>
						{t('common.back')}
					</button>
					<button
						type="button"
						class="w-2/3 rounded-md bg-gold px-4 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
						disabled={testState !== 'ok'}
						onclick={() => (step = 3)}
					>
						{t('setup.continue')}
					</button>
				</div>
			{:else}
				<h1 class="mb-1 text-xl font-semibold text-neutral-50">{t('setup.step_admin_title')}</h1>
				<p class="mb-6 text-sm text-neutral-400">{t('setup.step_admin_subtitle')}</p>

				<div class="flex flex-col gap-3">
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_admin_username')}
						<input type="text" class={inputClass} bind:value={adminUsername} />
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_admin_password')}
						<input type="password" class={inputClass} bind:value={adminPassword} />
						<span class="text-xs text-neutral-500">{t('setup.password_hint')}</span>
					</label>
					<label class="flex flex-col gap-1 text-sm text-neutral-300">
						{t('setup.field_confirm_password')}
						<input type="password" class={inputClass} bind:value={confirmPassword} />
					</label>
				</div>

				{#if confirmPassword.length > 0 && !passwordsMatch}
					<p class="mt-3 text-sm text-red-400">{t('setup.password_mismatch')}</p>
				{/if}

				{#if finishErrorCode}
					<p class="mt-3 text-sm text-red-400">{t(`errors.${finishErrorCode}`)}</p>
				{/if}

				<div class="mt-6 flex gap-3">
					<button
						type="button"
						class="w-1/3 rounded-md border border-neutral-700 px-4 py-2.5 font-medium text-neutral-200 hover:border-gold-dim"
						onclick={() => (step = 2)}
					>
						{t('common.back')}
					</button>
					<button
						type="button"
						class="w-2/3 rounded-md bg-gold px-4 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
						disabled={!adminStepValid || finishing}
						onclick={finishSetup}
					>
						{finishing ? t('setup.finishing') : t('setup.finish')}
					</button>
				</div>
			{/if}
		</div>
	</div>
</main>
