<script lang="ts">
    import { onMount } from 'svelte';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { api } from '$lib/playback/api';
    let rows = $state<{ username: string; plays: number; watched_seconds: number }[]>([]);
    let loading = $state(true);
    let failed = $state(false);
    onMount(async () => {
        try { rows = await api(getSessionCookieClientSide() ?? '', '/api/admin/stats/users'); }
        catch { failed = true; }
        finally { loading = false; }
    });
</script>
<main class="mx-auto max-w-4xl px-4 py-8 text-white">
    <h1 class="mb-4 text-2xl font-semibold">{t('player.stats')}</h1>
    <p class="mb-6 text-sm text-neutral-400">{t('player.stats_hint')}</p>
    {#if loading}<p>{t('common.loading')}</p>
    {:else if failed}<p role="alert">{t('player.load_failed')}</p>
    {:else if rows.length === 0}<p>{t('player.list_empty')}</p>
    {:else}
        <table class="w-full text-left"><thead><tr class="border-b border-white/20">
            <th class="p-3">{t('player.user')}</th><th class="p-3">{t('player.plays')}</th><th class="p-3">{t('player.minutes')}</th>
        </tr></thead><tbody>
            {#each rows as row}<tr class="border-b border-white/10"><td class="p-3">{row.username}</td><td class="p-3">{row.plays}</td><td class="p-3">{Math.round(row.watched_seconds / 60)}</td></tr>{/each}
        </tbody></table>
    {/if}
</main>
