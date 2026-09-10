<script lang="ts">
    import { onMount } from 'svelte';
    import { goto, invalidateAll } from '$app/navigation';
    import { resolve } from '$app/paths';
    import { page } from '$app/state';
    import { t, getLocale, SUPPORTED_LOCALES } from '$lib/i18n';
    import { getSessionCookieClientSide, clearSessionCookie } from '$lib/auth';
    import { accountRequest, accountErrorKey, type Account } from '$lib/users/api';
    let users = $state<Account[]>([]), selected = $state<Account | null>(null);
    let creating = $state(false), loaded = $state(false), busy = $state(false);
    let name = $state(''), admin = $state(false), disabled = $state(false), language = $state('');
    let password = $state(''), error = $state(''), saved = $state(false);
    let secret = $state(''), secretName = $state(''), reveal = $state(false), copied = $state(false), signedOut = $state(false);
    let token = '';
    onMount(async () => {
        token = getSessionCookieClientSide() ?? '';
        try { users = await accountRequest<Account[]>(token, '/api/admin/users'); loaded = true; }
        catch (e) { error = accountErrorKey(e); }
    });
    function edit(user: Account | null) {
        selected = user; creating = !user;
        name = user?.username ?? ''; admin = user?.is_admin ?? false; disabled = user?.disabled ?? false;
        language = user?.ui_language ?? getLocale(); password = ''; error = ''; saved = false;
    }
    function receipt(value: string, username: string) {
        secret = value; secretName = username; reveal = false; copied = false; password = '';
    }
    async function leave() { clearSessionCookie(); await goto(resolve('/login'), { invalidateAll: true }); }
    async function save(event: SubmitEvent) {
        event.preventDefault(); if (busy || signedOut) return;
        busy = true; error = ''; saved = false;
        try {
            if (creating) {
                const result = await accountRequest<{user: Account; temporary_password: string}>(token, '/api/admin/users', 'POST',
                    { username: name, is_admin: admin, ui_language: language, password: password || null });
                users = [...users, result.user]; edit(result.user); receipt(result.temporary_password, result.user.username);
            } else if (selected) {
                const result = await accountRequest<Account>(token, `/api/admin/users/${selected.id}`, 'PUT',
                    { username: name, is_admin: admin, disabled, ui_language: language });
                users = users.map(u => u.id === result.id ? result : u);
                const sessionRevoked = result.id === page.data.user?.id && (result.disabled || result.is_admin !== selected.is_admin);
                edit(result);
                if (sessionRevoked) { await leave(); return; }
                if (result.id === page.data.user?.id) await invalidateAll();
            }
            saved = true;
        } catch (e) { error = accountErrorKey(e); } finally { busy = false; }
    }
    async function reset(event: SubmitEvent) {
        event.preventDefault(); if (!selected || busy || signedOut) return;
        busy = true; error = ''; saved = false;
        try {
            const result = await accountRequest<{temporary_password: string}>(token, `/api/admin/users/${selected.id}/password`, 'POST', { password: password || null });
            receipt(result.temporary_password, selected.username);
            const updated = { ...selected, must_change_password: true };
            selected = updated;
            users = users.map(u => u.id === updated.id ? updated : u);
            signedOut = selected.id === page.data.user?.id;
        } catch (e) { error = accountErrorKey(e); } finally { busy = false; }
    }
    async function copy() {
        try { await navigator.clipboard.writeText(secret); copied = true; }
        catch { reveal = true; }
    }
</script>
<svelte:head><title>{t('accounts.title')} · LisekMovie</title></svelte:head>
<main class="mx-auto max-w-5xl px-4 py-8 text-white md:px-8">
    <div class="mb-6 flex items-center justify-between gap-4"><h1 class="text-2xl font-semibold">{t('accounts.title')}</h1><button class="primary" disabled={!loaded || busy || signedOut} onclick={() => edit(null)}>+ {t('accounts.add_user')}</button></div>
    <p class="mb-6 text-sm text-neutral-400">{t('accounts.creation_hint')}</p>
    {#if secret}
        <section class="mb-6 rounded-lg border border-gold/60 bg-surface p-5" aria-label={t('accounts.temporary_password')}>
            <h2 class="font-semibold">{t('accounts.temporary_password_for', { username: secretName })}</h2>
            <p class="my-2 text-sm text-neutral-300">{t('accounts.temporary_password_hint')}</p>
            <label>{t('accounts.temporary_password')}<input readonly type={reveal ? 'text' : 'password'} value={secret} autocomplete="off" /></label>
            <div class="mt-3 flex flex-wrap gap-3"><button onclick={() => reveal = !reveal}>{reveal ? t('accounts.hide') : t('accounts.show')}</button><button onclick={copy}>{copied ? t('accounts.copied') : t('accounts.copy_password')}</button>
                {#if signedOut}<button class="primary" onclick={leave}>{t('accounts.password_saved_sign_in_again')}</button>{:else}<button onclick={() => secret = ''}>{t('accounts.close')}</button>{/if}</div>
        </section>
    {/if}
    {#if error}<p role="alert" class="mb-5 rounded bg-red-950/40 p-4 text-red-200">{t(error)}</p>{/if}
    {#if saved}<p role="status" class="mb-4 text-gold">{t('accounts.saved')}</p>{/if}
    <div class="grid gap-6 md:grid-cols-[240px_1fr]">
        <nav aria-label={t('accounts.user_list')} class="flex flex-col gap-2">
            {#if !loaded}<p>{t('accounts.loading')}</p>{/if}
            {#each users as user (user.id)}
                <button disabled={busy || signedOut} onclick={() => edit(user)} aria-current={selected?.id === user.id ? 'true' : undefined} class="text-left {selected?.id === user.id ? 'border-gold bg-white/5' : ''}">
                    <span class="block break-all font-medium">{user.username}</span>
                    <span class="text-xs text-neutral-400">{user.is_admin ? t('accounts.administrator') : t('accounts.user')} · {user.disabled ? t('accounts.disabled') : user.must_change_password ? t('accounts.password_change_required') : t('accounts.active')}</span>
                </button>
            {/each}
        </nav>
        {#if creating || selected}
            <div class="rounded-lg border border-white/10 bg-surface/50 p-6">
                <h2 class="mb-5 text-xl">{creating ? t('accounts.add_user') : selected?.username}</h2>
                <form onsubmit={save} class="flex flex-col gap-4">
                    <fieldset disabled={busy || signedOut} class="flex flex-col gap-4">
                        <label>{t('accounts.username')}<input required maxlength="128" autocomplete="off" bind:value={name} /></label>
                        <label>{t('accounts.interface_language')}<select bind:value={language}>{#each SUPPORTED_LOCALES as locale}<option value={locale}>{t(`languages.${locale}`)}</option>{/each}</select></label>
                        <label class="check"><input type="checkbox" bind:checked={admin} />{t('accounts.allow_server_management_administrator')}</label>
                        {#if !creating}<label class="check"><input type="checkbox" bind:checked={disabled} />{t('accounts.disable_account')}</label><p class="text-xs text-neutral-400">{t('accounts.access_hint')}</p>{/if}
                        {#if creating}<label>{t('accounts.custom_temporary_password_optional')}<input type="password" minlength="8" maxlength="1024" autocomplete="new-password" bind:value={password} /></label><p class="text-xs text-neutral-400">{t('accounts.generate_password_hint')}</p>{/if}
                        <div class="flex gap-3"><button class="primary">{creating ? t('accounts.create_account') : t('accounts.save_profile')}</button><button type="button" onclick={() => {selected = null; creating = false; password = '';}}>{t('accounts.cancel')}</button></div>
                    </fieldset>
                </form>
                {#if selected && !creating}
                    <form onsubmit={reset} class="mt-8 border-t border-white/10 pt-6">
                        <fieldset disabled={busy || signedOut} class="flex flex-col gap-4">
                            <legend class="mb-3 text-lg">{t('accounts.password_reset')}</legend>
                            <p class="text-sm text-neutral-400">{t('accounts.reset_password_hint')}</p>
                            <label>{t('accounts.custom_temporary_password_optional')}<input type="password" minlength="8" maxlength="1024" autocomplete="new-password" bind:value={password} /></label>
                            <button>{password ? t('accounts.set_temporary_password') : t('accounts.generate_new_temporary_password')}</button>
                        </fieldset>
                    </form>
                {/if}
            </div>
        {:else}<p class="rounded-lg border border-dashed border-white/10 p-8 text-neutral-400">{t('accounts.select_account_hint')}</p>{/if}
    </div>
</main>
<style>
    button { border:1px solid #525252; border-radius:0.4rem; padding:0.6rem 0.8rem; }
    button:disabled,fieldset:disabled { opacity:0.5; }
    .primary { background:#d5ad42; border-color:#d5ad42; color:#111; }
    label { font-size:0.9rem; }
    input:not([type=checkbox]),select { display:block; width:100%; margin-top:0.4rem; border:1px solid #525252; border-radius:0.4rem; padding:0.65rem; background:#171717; color:white; }
    .check { display:flex; align-items:center; gap:0.65rem; }
    button:focus-visible,input:focus-visible,select:focus-visible { outline:2px solid #d5ad42; outline-offset:2px; }
</style>
