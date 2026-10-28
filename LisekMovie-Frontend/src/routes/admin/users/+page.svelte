<script lang="ts">
    import { onMount } from 'svelte';
    import { goto, invalidateAll } from '$app/navigation';
    import { resolve } from '$app/paths';
    import { page } from '$app/state';
    import { t, getLocale, SUPPORTED_LOCALES } from '$lib/i18n';
    import { getSessionCookieClientSide, clearSessionCookie } from '$lib/auth';
    import { accountRequest, accountErrorKey, type Account } from '$lib/users/api';
    import { listRestricted } from '$lib/users/restrictions';
    import { fetchLibraries, type Library } from '$lib/admin/libraries';
    import UserRestrictions from '$lib/components/UserRestrictions.svelte';
    import Icon from '$lib/components/Icon.svelte';
    import { SvelteSet } from 'svelte/reactivity';
    import Avatar from '$lib/components/Avatar.svelte';
    import { relativeTime } from '$lib/users/profile';
    /** Faza 19: live filter by name (debounced) and presence refresh. */
    const SEARCH_DEBOUNCE_MS = 200, PRESENCE_REFRESH_MS = 30_000;
    let query = $state(''), debounced = $state(''), now = $state(Date.now());
    $effect(() => {
        const value = query;
        const timer = setTimeout(() => debounced = value, SEARCH_DEBOUNCE_MS);
        return () => clearTimeout(timer);
    });
    let users = $state<Account[]>([]), selected = $state<Account | null>(null);
    let creating = $state(false), loaded = $state(false), busy = $state(false);
    let name = $state(''), admin = $state(false), disabled = $state(false), language = $state('');
    let password = $state(''), error = $state(''), saved = $state(false);
    let secret = $state(''), secretName = $state(''), reveal = $state(false), copied = $state(false), signedOut = $state(false);
    let token = $state('');
    /** Faza 6: accounts with parental controls (lock in the list) and the
     * libraries the restrictions editor offers. */
    const restricted = new SvelteSet<string>();
    let libraries = $state<Library[]>([]);
    const filtered = $derived.by(() => {
        const needle = debounced.trim().toLocaleLowerCase();
        return needle ? users.filter(u => u.username.toLocaleLowerCase().includes(needle)) : users;
    });
    const onlineCount = $derived(users.filter(u => u.online).length);
    function presence(user: Account): string {
        if (user.online) return user.now_playing ? t('accounts.now_playing', { title: user.now_playing }) : t('profile.online');
        return user.last_seen_at ? t('accounts.last_seen', { time: relativeTime(user.last_seen_at, getLocale(), now) }) : t('accounts.never_seen');
    }
    /** Avatars and online status change on their own; account fields are left as edited here. */
    async function refreshPresence() {
        try {
            const fresh = new Map((await accountRequest<Account[]>(token, '/api/admin/users')).map(u => [u.id, u]));
            users = users.map(u => { const f = fresh.get(u.id); return f ? { ...u, avatar_url: f.avatar_url, online: f.online, last_seen_at: f.last_seen_at, now_playing: f.now_playing } : u; });
        } catch { /* The next refresh tries again. */ }
        now = Date.now();
    }
    onMount(() => {
        const timer = setInterval(() => { if (loaded && document.visibilityState === 'visible') void refreshPresence(); }, PRESENCE_REFRESH_MS);
        return () => clearInterval(timer);
    });
    onMount(async () => {
        token = getSessionCookieClientSide() ?? '';
        try { users = await accountRequest<Account[]>(token, '/api/admin/users'); loaded = true; now = Date.now(); }
        catch (e) { error = accountErrorKey(e); }
        listRestricted(token).then(r => r.forEach(x => restricted.add(x.user_id))).catch(() => {});
        fetchLibraries(token).then(l => libraries = l).catch(() => {});
    });
    function markRestricted(id: string, on: boolean) {
        if (on) restricted.add(id); else restricted.delete(id);
    }
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
                users = [...users, { ...result.user, avatar_url: null, online: false, last_seen_at: null, now_playing: null }]; edit(result.user); receipt(result.temporary_password, result.user.username);
            } else if (selected) {
                const result = await accountRequest<Account>(token, `/api/admin/users/${selected.id}`, 'PUT',
                    { username: name, is_admin: admin, disabled, ui_language: language });
                users = users.map(u => u.id === result.id ? { ...u, ...result } : u);
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
    <div class="grid gap-6 md:grid-cols-[300px_1fr]">
        <nav aria-label={t('accounts.user_list')} class="flex flex-col gap-2">
            <div class="relative">
                <Icon name="search" class="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-neutral-500" />
                <input type="search" class="search" bind:value={query} placeholder={t('accounts.search_placeholder')} aria-label={t('accounts.search_placeholder')} autocomplete="off" />
            </div>
            {#if loaded}<p class="flex items-center gap-1.5 px-1 text-xs text-neutral-400"><span class="size-2 rounded-full bg-emerald-500"></span>{t('accounts.online_count', { online: String(onlineCount), total: String(users.length) })}</p>{/if}
            {#if !loaded}<p>{t('accounts.loading')}</p>{/if}
            {#each filtered as user (user.id)}
                <button disabled={busy || signedOut} onclick={() => edit(user)} aria-current={selected?.id === user.id ? 'true' : undefined} class="user-row {selected?.id === user.id ? 'is-selected' : ''}">
                    <Avatar username={user.username} url={user.avatar_url} online={user.online ?? false} class="size-11 text-base" />
                    <span class="flex min-w-0 flex-1 flex-col">
                        <span class="flex items-center gap-1.5 font-medium"><span class="truncate">{user.username}</span>{#if restricted.has(user.id) && !user.is_admin}<span class="text-gold" title={t('restrictions.restricted_badge')}><Icon name="lock" class="size-3.5" /></span>{/if}{#if user.is_admin}<span class="badge">{t('accounts.administrator')}</span>{/if}</span>
                        <span class="truncate text-xs {user.online ? 'text-emerald-400' : 'text-neutral-400'}">{presence(user)}</span>
                        <span class="truncate text-xs text-neutral-500">{user.disabled ? t('accounts.disabled') : user.must_change_password ? t('accounts.password_change_required') : t('accounts.active')}</span>
                    </span>
                </button>
            {:else}
                {#if loaded}<p class="rounded-lg border border-dashed border-white/10 p-4 text-sm text-neutral-400">{t('accounts.no_matches')}</p>{/if}
            {/each}
        </nav>
        {#if creating || selected}
            <div class="rounded-lg border border-white/10 bg-surface/50 p-6">
                {#if selected && !creating}
                    {@const listed = users.find(u => u.id === selected?.id)}
                    <div class="mb-5 flex items-center gap-4">
                        <Avatar username={selected.username} url={listed?.avatar_url} online={listed?.online ?? false} class="size-16 text-2xl" />
                        <div class="min-w-0"><h2 class="truncate text-xl">{selected.username}</h2><p class="text-sm {listed?.online ? 'text-emerald-400' : 'text-neutral-400'}">{listed ? presence(listed) : ''}</p></div>
                    </div>
                {:else}<h2 class="mb-5 text-xl">{t('accounts.add_user')}</h2>{/if}
                <form onsubmit={save} class="flex flex-col gap-4">
                    <fieldset disabled={busy || signedOut} class="flex flex-col gap-4">
                        <label>{t('accounts.username')}<input required maxlength="128" autocomplete="off" bind:value={name} /></label>
                        <label>{t('accounts.interface_language')}<select bind:value={language}>{#each SUPPORTED_LOCALES as locale (locale)}<option value={locale}>{t(`languages.${locale}`)}</option>{/each}</select></label>
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
                    {#key selected.id}
                        <UserRestrictions userId={selected.id} isAdmin={selected.is_admin} {libraries} {token} disabled={signedOut} onchange={(on) => selected && markRestricted(selected.id, on)} />
                    {/key}
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
    .user-row { display:flex; align-items:center; gap:0.8rem; width:100%; text-align:left; border-color:rgb(255 255 255 / 0.08); border-radius:0.75rem; background:rgb(255 255 255 / 0.02); transition:background-color 120ms, border-color 120ms; }
    .user-row:not(:disabled):hover { background:rgb(255 255 255 / 0.06); border-color:rgb(255 255 255 / 0.18); }
    .user-row.is-selected { border-color:#d5ad42; background:rgb(213 173 66 / 0.08); }
    .badge { flex-shrink:0; border-radius:9999px; background:rgb(213 173 66 / 0.15); padding:0.05rem 0.45rem; font-size:0.65rem; font-weight:600; color:#d5ad42; }
    input.search { margin-top:0; padding-left:2.2rem; border-radius:0.6rem; }
    button:focus-visible,input:focus-visible,select:focus-visible { outline:2px solid #d5ad42; outline-offset:2px; }
</style>
