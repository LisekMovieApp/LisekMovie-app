<!--
  Sign-in form. The only page a signed-out browser can ever see besides
  /setup — the root layout guard (`+layout.ts`) redirects here before any
  other page renders, and back to "/" the moment a session cookie proves
  valid. Styled to match /setup: fullscreen, minimalist, black background,
  gold accents.
-->
<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { setSessionCookie } from '$lib/auth';
	import logoText from '$lib/assets/logo_text.svg';

	interface AuthErrorBody {
		error_code: string;
		message: string;
	}

	const inputClass =
		'rounded-md border border-neutral-700 bg-black px-3 py-2 text-neutral-100 focus:border-gold';

	let username = $state('');
	let password = $state('');
	let signingIn = $state(false);
	let errorCode = $state<string | null>(null);

	let canSubmit = $derived(username.trim().length > 0 && password.length > 0 && !signingIn);

	async function handleSubmit(event: SubmitEvent) {
		event.preventDefault();
		if (!canSubmit) return;

		signingIn = true;
		errorCode = null;

		try {
			const response = await fetch('/api/auth/login', {
				method: 'POST',
				headers: { 'Content-Type': 'application/json' },
				body: JSON.stringify({ username, password, client: 'web', device: 'Browser' })
			});

			if (!response.ok) {
				const body = (await response.json().catch(() => null)) as AuthErrorBody | null;
				errorCode = body?.error_code ?? 'internal_error';
				signingIn = false;
				return;
			}

			const { token, user } = (await response.json()) as { token: string; user: { must_change_password: boolean } };
			setSessionCookie(token);
			await goto(user.must_change_password ? resolve('/change-password') : resolve('/'), { invalidateAll: true });
		} catch {
			errorCode = 'network_error';
			signingIn = false;
		}
	}
</script>

<main class="flex min-h-screen flex-col items-center justify-center bg-base px-6 py-12">
	<div class="w-full max-w-sm">
		<div class="mb-8 flex flex-col items-center gap-2 text-center">
			<img src={logoText} alt="LisekMovie" class="w-40" />
			<p class="text-sm text-neutral-400">{t('auth.login_subtitle')}</p>
		</div>

		<form
			class="rounded-lg border border-surface bg-surface/60 p-8"
			onsubmit={handleSubmit}
			autocomplete="on"
		>
			<div class="flex flex-col gap-3">
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('auth.field_username')}
					<input type="text" class={inputClass} bind:value={username} autocomplete="username" />
				</label>
				<label class="flex flex-col gap-1 text-sm text-neutral-300">
					{t('auth.field_password')}
					<input
						type="password"
						class={inputClass}
						bind:value={password}
						autocomplete="current-password"
					/>
				</label>
			</div>

			{#if errorCode}
				<p class="mt-4 text-sm text-red-400">{t(`errors.${errorCode}`)}</p>
			{/if}

			<button
				type="submit"
				class="mt-6 w-full rounded-md bg-gold px-4 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
				disabled={!canSubmit}
			>
				{signingIn ? t('auth.signing_in') : t('auth.sign_in')}
			</button>
		</form>
	</div>
</main>
