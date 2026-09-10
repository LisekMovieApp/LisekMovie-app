<script lang="ts">
    import { goto } from '$app/navigation';
    import { resolve } from '$app/paths';
    import { page } from '$app/state';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide, setSessionCookie, clearSessionCookie, buildAuthHeader } from '$lib/auth';
    import { accountRequest, accountErrorKey } from '$lib/users/api';
    let required = $derived(page.data.user?.must_change_password ?? false);
    let password = $state(''), repeat = $state(''), current = $state(''), error = $state(''), busy = $state(false);
    async function save(event: SubmitEvent) {
        event.preventDefault(); if (busy) return;
        if (password !== repeat) { error = 'password_change.passwords_mismatch'; return; }
        busy = true; error = '';
        try {
            const result = await accountRequest<{ token: string }>(getSessionCookieClientSide() ?? '', '/api/auth/password', 'POST', { new_password: password, current_password: required ? null : current });
            password = repeat = current = ''; setSessionCookie(result.token);
            await goto(resolve('/'), { invalidateAll: true });
        } catch (e) { error = accountErrorKey(e); } finally { busy = false; }
    }
    async function logout() {
        const token = getSessionCookieClientSide();
        try { if (token) await fetch('/api/auth/logout', { method:'POST', headers:buildAuthHeader(token) }); }
        finally { clearSessionCookie(); await goto(resolve('/login'), { invalidateAll:true }); }
    }
</script>
<svelte:head><title>{t('password_change.page_title')} · LisekMovie</title></svelte:head>
<main class="flex min-h-screen items-center justify-center bg-base px-5 py-28 text-white">
    <section class="w-full max-w-md rounded-xl border border-white/10 bg-surface p-8">
        <h1 class="mb-4 text-2xl font-semibold">{t('password_change.title')}</h1>
        <p class="mb-6 text-sm text-neutral-400">{required ? t('password_change.temporary_password_hint') : t('password_change.session_revocation_hint')}</p>
        <form onsubmit={save} class="flex flex-col gap-4">
            {#if !required}<label>{t('password_change.current_password')}<input type="password" required autocomplete="current-password" maxlength="1024" bind:value={current} /></label>{/if}
            <label>{t('password_change.new_password')}<input type="password" required minlength="8" maxlength="1024" autocomplete="new-password" bind:value={password} /></label>
            <label>{t('password_change.repeat_new_password')}<input type="password" required minlength="8" maxlength="1024" autocomplete="new-password" bind:value={repeat} /></label>
            <p class="text-xs text-neutral-400">{t('password_change.password_hint')}</p>
            {#if error}<p role="alert" class="text-sm text-red-300">{t(error)}</p>{/if}
            <button disabled={busy} class="rounded bg-gold p-3 font-medium text-black disabled:opacity-50">{busy ? t('password_change.saving') : t('password_change.save_password_and_continue')}</button>
        </form>
        <button disabled={busy} onclick={logout} class="mt-5 text-sm text-neutral-400 underline">{t('password_change.sign_out')}</button>
    </section>
</main>
<style>
    label { font-size:0.9rem; }
    input { display:block; width:100%; margin-top:0.4rem; border:1px solid #525252; border-radius:0.4rem; padding:0.7rem; background:#171717; }
    input:focus-visible,button:focus-visible { outline:2px solid #d5ad42; outline-offset:2px; }
</style>
