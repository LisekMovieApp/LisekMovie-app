<script lang="ts">
    import { t } from '$lib/i18n';
    import { api, type MediaState } from '$lib/playback/api';
    let { itemId, token }: { itemId: string; token: string } = $props();
    let savedState = $state<MediaState | null>(null);
    let busy = $state(false);
    let failed = $state(false);

    $effect(() => {
        const id = itemId;
        const sessionToken = token;
        let active = true;
        savedState = null;
        failed = false;
        busy = false;
        api<MediaState>(sessionToken, `/api/media/${encodeURIComponent(id)}/state`)
            .then(result => { if (active) savedState = result; })
            .catch(() => { if (active) failed = true; });
        return () => { active = false; };
    });
    async function toggle(key: 'watched' | 'favorite') {
        if (!savedState || busy) return;
        const id = itemId;
        const value = !savedState[key];
        busy = true;
        failed = false;
        try {
            await api<void>(token, `/api/media/${encodeURIComponent(id)}/state`, 'PUT', { [key]: value });
            if (itemId === id && savedState) savedState = { ...savedState, [key]: value };
        } catch { if (itemId === id) failed = true; }
        finally { if (itemId === id) busy = false; }
    }
</script>
<div class="flex flex-wrap gap-3">
    {#if savedState}
        <button class="rounded-md border border-white/20 px-4 py-2 text-sm text-white hover:border-gold disabled:opacity-50" disabled={busy} aria-pressed={savedState.favorite} onclick={() => toggle('favorite')}>{t(savedState.favorite ? 'player.unfavorite' : 'player.favorite')}</button>
        <button class="rounded-md border border-white/20 px-4 py-2 text-sm text-white hover:border-gold disabled:opacity-50" disabled={busy} aria-pressed={savedState.watched} onclick={() => toggle('watched')}>{t(savedState.watched ? 'player.unwatched' : 'player.watched')}</button>
    {/if}
    {#if failed}<p role="alert" class="text-sm text-gold">{t('player.save_failed')}</p>{/if}
</div>
