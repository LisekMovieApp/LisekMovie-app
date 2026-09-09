<script lang="ts">
    import { onMount } from 'svelte';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { api } from '$lib/playback/api';
    let games = $state<{ game_key: string; name: string; enabled: boolean }[]>([]);
    let failed = $state(false);
    let busy = $state(false);
    let token = '';
    onMount(async () => {
        token = getSessionCookieClientSide() ?? '';
        try { games = await api(token, '/api/admin/games'); } catch { failed = true; }
    });
    async function toggle(key: string, enabled: boolean) {
        busy = true;
        failed = false;
        try {
            await api(token, '/api/admin/games/' + encodeURIComponent(key), 'PUT', { enabled });
            games = games.map(game => game.game_key === key ? { ...game, enabled } : game);
            window.dispatchEvent(new Event('lisek-games-changed'));
        } catch { failed = true; }
        finally { busy = false; }
    }
</script>
<main class="mx-auto max-w-3xl px-4 py-8 text-white">
    <h1 class="mb-6 text-2xl">{t('nav.games')}</h1>
    {#each games as game}
        <div class="flex items-center justify-between rounded-lg bg-surface p-4">
            <span>{game.name}</span>
            <button class="rounded-md border border-gold px-4 py-2 text-gold disabled:opacity-50" disabled={busy} aria-pressed={game.enabled} onclick={() => toggle(game.game_key, !game.enabled)}>{t(game.enabled ? 'games.disable' : 'games.enable')}</button>
        </div>
    {/each}
    {#if failed}<p role="alert" class="mt-4">{t('player.save_failed')}</p>{/if}
</main>
