<script lang="ts">
    import PlaybackSettings from '$lib/components/PlaybackSettings.svelte';
    import HardwarePanel from '$lib/components/HardwarePanel.svelte';
    import { onMount } from 'svelte';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { api } from '$lib/playback/api';
    let enabled = $state(true), width = $state(1920), height = $state(1080);
    let bitrate = $state<number | undefined>();
    let loaded = $state(false), failed = $state(false), saved = $state(false);
    let token = '';
    let settingsVersion = $state(0);
    onMount(async () => {
        token = getSessionCookieClientSide() ?? '';
        try {
            const config = await api<{ enabled: boolean; max_width: number; max_height: number; max_bitrate_kbps: number | null }>(token, '/api/admin/playback/safety');
            enabled = config.enabled; width = config.max_width; height = config.max_height; bitrate = config.max_bitrate_kbps ?? undefined;
            loaded = true;
        } catch { failed = true; }
    });
    async function save(event: SubmitEvent) {
        event.preventDefault(); failed = false; saved = false;
        try { await api(token, '/api/admin/playback/safety', 'PUT', { enabled, max_width: width, max_height: height, max_bitrate_kbps: bitrate ?? null }); saved = true; }
        catch { failed = true; }
    }
</script>
<main class="mx-auto max-w-4xl px-4 py-8 text-white">
    <HardwarePanel onsaved={() => settingsVersion++} />
    {#key settingsVersion}<PlaybackSettings />{/key}
    <h2 class="mb-3 text-2xl">{t('player.safety')}</h2>
    <p class="mb-6 text-sm text-neutral-400">{t('player.safety_hint')}</p>
    {#if loaded}
        <form onsubmit={save} class="flex flex-col gap-4">
            <label class="flex gap-3"><input type="checkbox" bind:checked={enabled} />{t('player.safety_enabled')}</label>
            <label>{t('player.max_width')}<input class="mt-1 w-full rounded bg-surface p-3" type="number" min="1" max="16384" required bind:value={width} /></label>
            <label>{t('player.max_height')}<input class="mt-1 w-full rounded bg-surface p-3" type="number" min="1" max="16384" required bind:value={height} /></label>
            <label>{t('player.max_bitrate')}<input class="mt-1 w-full rounded bg-surface p-3" type="number" min="1" bind:value={bitrate} /></label>
            <button class="rounded bg-gold px-4 py-3 text-black">{t('player.save')}</button>
        </form>
    {/if}
    {#if failed}<p role="alert" class="mt-4">{t('player.save_failed')}</p>{/if}
    {#if saved}<p role="status" class="mt-4 text-gold">{t('player.saved')}</p>{/if}
</main>
