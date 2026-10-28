<!--
  Profile settings (Faza 19), for every signed-in account: the avatar
  (upload / remove), the password (the same secured endpoint as
  /change-password: current password required, every other session signed
  out) and the account's own interface language, with a note that library
  metadata stays in the server's default language.
-->
<script lang="ts">
	import { invalidateAll } from '$app/navigation';
	import { page } from '$app/state';
	import { t, SUPPORTED_LOCALES, setLocaleCookie, type Locale } from '$lib/i18n';
	import { getSessionCookieClientSide, setSessionCookie } from '$lib/auth';
	import { accountRequest, accountErrorKey } from '$lib/users/api';
	import {
		AVATAR_TYPES,
		avatarFileError,
		deleteAvatar,
		profileErrorKey,
		setLanguage,
		uploadAvatar
	} from '$lib/users/profile';
	import { showToast } from '$lib/toast/toast.svelte';
	import Avatar from '$lib/components/Avatar.svelte';
	import Icon from '$lib/components/Icon.svelte';

	const user = $derived(page.data.user);

	// Avatar
	let fileInput: HTMLInputElement | undefined = $state();
	let avatarBusy = $state(false);
	let preview = $state<string | null>(null);

	async function onFile(event: Event) {
		const input = event.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file || avatarBusy) return;
		const problem = avatarFileError(file);
		if (problem) {
			showToast('error', t(problem));
			return;
		}
		avatarBusy = true;
		preview = URL.createObjectURL(file);
		try {
			await uploadAvatar(getSessionCookieClientSide() ?? '', file);
			await invalidateAll();
			showToast('success', t('profile.avatar.saved'));
		} catch (error) {
			showToast('error', t(profileErrorKey(error)));
		} finally {
			if (preview) URL.revokeObjectURL(preview);
			preview = null;
			avatarBusy = false;
		}
	}

	async function removeAvatar() {
		if (avatarBusy) return;
		avatarBusy = true;
		try {
			await deleteAvatar(getSessionCookieClientSide() ?? '');
			await invalidateAll();
			showToast('success', t('profile.avatar.removed'));
		} catch (error) {
			showToast('error', t(profileErrorKey(error)));
		} finally {
			avatarBusy = false;
		}
	}

	// Password
	let current = $state('');
	let password = $state('');
	let repeat = $state('');
	let passwordBusy = $state(false);
	let passwordError = $state('');
	const mismatch = $derived(repeat.length > 0 && password !== repeat);

	async function savePassword(event: SubmitEvent) {
		event.preventDefault();
		if (passwordBusy) return;
		if (password !== repeat) {
			passwordError = 'password_change.passwords_mismatch';
			return;
		}
		passwordBusy = true;
		passwordError = '';
		try {
			const result = await accountRequest<{ token: string }>(
				getSessionCookieClientSide() ?? '',
				'/api/auth/password',
				'POST',
				{ current_password: current, new_password: password }
			);
			setSessionCookie(result.token);
			current = password = repeat = '';
			showToast('success', t('profile.password.saved'));
			await invalidateAll();
		} catch (error) {
			passwordError = accountErrorKey(error);
		} finally {
			passwordBusy = false;
		}
	}

	// Language
	let languageBusy = $state(false);

	async function changeLanguage(event: Event) {
		const language = (event.currentTarget as HTMLSelectElement).value as Locale;
		if (!user || language === user.ui_language) return;
		languageBusy = true;
		try {
			await setLanguage(getSessionCookieClientSide() ?? '', language);
			setLocaleCookie(language);
			await invalidateAll();
			showToast('success', t('profile.language.saved'));
		} catch (error) {
			showToast('error', t(profileErrorKey(error)));
		} finally {
			languageBusy = false;
		}
	}
</script>

<svelte:head><title>{t('profile.title')} · LisekMovie</title></svelte:head>

<main class="mx-auto max-w-3xl px-4 pt-24 pb-16 text-white md:px-8">
	<header class="mb-8">
		<h1 class="text-3xl font-semibold tracking-tight">{t('profile.title')}</h1>
		<p class="mt-1 text-sm text-neutral-400">{t('profile.subtitle')}</p>
	</header>

	{#if user}
		<!-- Avatar -->
		<section class="card" aria-labelledby="avatar-heading">
			<div class="flex flex-col items-center gap-6 sm:flex-row sm:items-center">
				<div class="group relative">
					{#if preview}
						<img
							src={preview}
							alt=""
							class="size-28 rounded-full object-cover opacity-60 ring-2 ring-gold"
						/>
					{:else}
						<Avatar username={user.username} url={user.avatar_url} class="size-28 text-4xl" />
					{/if}
					<button
						type="button"
						disabled={avatarBusy}
						onclick={() => fileInput?.click()}
						aria-label={t('profile.avatar.change')}
						class="absolute inset-0 flex flex-col items-center justify-center gap-1 rounded-full bg-black/60 text-xs font-medium text-white transition-opacity disabled:cursor-wait {avatarBusy
							? 'opacity-100'
							: 'opacity-0 group-hover:opacity-100 focus-visible:opacity-100'}"
					>
						{#if avatarBusy}
							<span class="size-6 animate-spin rounded-full border-2 border-white/30 border-t-white"
							></span>
						{:else}
							<Icon name="camera" class="size-6" />
							{t('profile.avatar.change')}
						{/if}
					</button>
					{#if user.avatar_url && !avatarBusy}
						<button
							type="button"
							onclick={removeAvatar}
							aria-label={t('profile.avatar.remove')}
							title={t('profile.avatar.remove')}
							class="absolute -top-1 -right-1 flex size-8 items-center justify-center rounded-full border border-white/15 bg-surface text-neutral-300 opacity-0 shadow-lg transition-opacity group-hover:opacity-100 hover:text-red-300 focus-visible:opacity-100"
						>
							<Icon name="trash" class="size-4" />
						</button>
					{/if}
				</div>
				<div class="flex-1 text-center sm:text-left">
					<h2 id="avatar-heading" class="text-xl font-semibold">{user.username}</h2>
					<p
						class="mt-0.5 flex items-center justify-center gap-1 text-sm font-light text-neutral-400 sm:justify-start"
					>
						{#if user.is_admin}<Icon name="shield" class="size-4 text-gold" />{/if}
						{t('profile.menu.role', {
							role: user.is_admin ? t('accounts.administrator') : t('accounts.user')
						})}
					</p>
					<div class="mt-4 flex flex-wrap justify-center gap-2 sm:justify-start">
						<button
							type="button"
							class="btn primary"
							disabled={avatarBusy}
							onclick={() => fileInput?.click()}
						>
							<Icon name="camera" class="size-4" />
							{t('profile.avatar.change')}
						</button>
						<button
							type="button"
							class="btn danger"
							disabled={avatarBusy || !user.avatar_url}
							onclick={removeAvatar}
						>
							<Icon name="trash" class="size-4" />
							{t('profile.avatar.remove')}
						</button>
					</div>
					<p class="mt-2 text-xs text-neutral-500">{t('profile.avatar.hint')}</p>
					<input
						bind:this={fileInput}
						type="file"
						accept={AVATAR_TYPES.join(',')}
						class="hidden"
						onchange={onFile}
						aria-label={t('profile.avatar.change')}
					/>
				</div>
			</div>
		</section>

		<!-- Password -->
		<section class="card" aria-labelledby="password-heading">
			<h2 id="password-heading" class="card-title">
				<Icon name="key" class="size-5 text-gold" />
				{t('profile.password.title')}
			</h2>
			<p class="mb-5 text-sm text-neutral-400">{t('password_change.session_revocation_hint')}</p>
			<form onsubmit={savePassword} class="grid gap-4 sm:grid-cols-2">
				<label class="sm:col-span-2">
					{t('profile.password.current')}
					<input
						type="password"
						required
						autocomplete="current-password"
						maxlength="1024"
						bind:value={current}
					/>
				</label>
				<label>
					{t('profile.password.new')}
					<input
						type="password"
						required
						minlength="8"
						maxlength="1024"
						autocomplete="new-password"
						bind:value={password}
					/>
				</label>
				<label>
					{t('profile.password.confirm')}
					<input
						type="password"
						required
						minlength="8"
						maxlength="1024"
						autocomplete="new-password"
						bind:value={repeat}
						aria-invalid={mismatch}
						class={mismatch ? 'invalid' : ''}
					/>
				</label>
				<p class="text-xs text-neutral-500 sm:col-span-2">
					{mismatch ? t('password_change.passwords_mismatch') : t('password_change.password_hint')}
				</p>
				{#if passwordError}<p role="alert" class="text-sm text-red-300 sm:col-span-2">
						{t(passwordError)}
					</p>{/if}
				<div class="sm:col-span-2">
					<button class="btn primary" disabled={passwordBusy || mismatch}>
						{passwordBusy ? t('password_change.saving') : t('profile.password.save')}
					</button>
				</div>
			</form>
		</section>

		<!-- Language -->
		<section class="card" aria-labelledby="language-heading">
			<h2 id="language-heading" class="card-title">
				<Icon name="globe" class="size-5 text-gold" />
				{t('profile.language.title')}
			</h2>
			<label class="max-w-xs">
				{t('profile.language.label')}
				<select value={user.ui_language} disabled={languageBusy} onchange={changeLanguage}>
					{#each SUPPORTED_LOCALES as locale (locale)}
						<option value={locale}>{t(`languages.${locale}`)}</option>
					{/each}
				</select>
			</label>
			<div
				role="note"
				class="mt-4 flex gap-3 rounded-lg border border-gold/30 bg-gold/10 p-4 text-sm text-neutral-200"
			>
				<Icon name="info" class="mt-0.5 size-5 shrink-0 text-gold" />
				<p>
					{t('profile.language.server_default', {
						language: t(`languages.${user.server_language}`)
					})}
				</p>
			</div>
		</section>
	{/if}
</main>

<style>
	.card {
		margin-bottom: 1.5rem;
		border: 1px solid rgb(255 255 255 / 0.1);
		border-radius: 1rem;
		background: linear-gradient(180deg, rgb(255 255 255 / 0.04), rgb(255 255 255 / 0.015));
		padding: 1.5rem;
	}
	.card-title {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin-bottom: 0.5rem;
		font-size: 1.15rem;
		font-weight: 600;
	}
	label {
		display: block;
		font-size: 0.9rem;
		color: rgb(212 212 212);
	}
	input:not([type='file']),
	select {
		display: block;
		width: 100%;
		margin-top: 0.4rem;
		border: 1px solid #404040;
		border-radius: 0.5rem;
		padding: 0.65rem 0.75rem;
		background: #111;
		color: white;
	}
	input.invalid {
		border-color: rgb(252 165 165 / 0.8);
	}
	input:focus-visible,
	select:focus-visible,
	button:focus-visible {
		outline: 2px solid var(--color-gold);
		outline-offset: 2px;
	}
	.btn {
		display: inline-flex;
		align-items: center;
		gap: 0.45rem;
		border: 1px solid #525252;
		border-radius: 0.5rem;
		padding: 0.55rem 0.9rem;
		font-size: 0.875rem;
		font-weight: 500;
		transition:
			background-color 120ms,
			border-color 120ms,
			color 120ms;
	}
	.btn:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}
	.btn.primary {
		border-color: var(--color-gold);
		background: var(--color-gold);
		color: #111;
	}
	.btn.primary:not(:disabled):hover {
		background: var(--color-gold-dim);
	}
	.btn.danger:not(:disabled):hover {
		border-color: rgb(252 165 165 / 0.6);
		color: rgb(254 202 202);
	}
</style>
