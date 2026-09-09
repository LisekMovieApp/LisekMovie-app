<script lang="ts">
    import { onMount } from 'svelte';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { api } from '$lib/playback/api';
    interface Endpoint { id: string; name: string; url: string; enabled: boolean; destination_type: string; }
    interface Delivery { id: string; name: string; status: string; attempt_count: number; response_code: number | null; }
    let endpoints = $state<Endpoint[]>([]), deliveries = $state<Delivery[]>([]);
    let name = $state(''), url = $state(''), destination = $state('generic_json'), enabled = $state(false);
    let failed = $state(false), busy = $state(false);
    let token = '';
    async function load() {
        [endpoints, deliveries] = await Promise.all([api<Endpoint[]>(token, '/api/admin/webhooks'), api<Delivery[]>(token, '/api/admin/webhooks/deliveries')]);
    }
    onMount(async () => { token = getSessionCookieClientSide() ?? ''; try { await load(); } catch { failed = true; } });
    async function create(event: SubmitEvent) {
        event.preventDefault(); busy = true; failed = false;
        try { await api(token, '/api/admin/webhooks', 'POST', { name, url, destination_type: destination, enabled, subscribed_events: ['item_added'] }); name=''; url=''; enabled=false; await load(); }
        catch { failed = true; } finally { busy = false; }
    }
    async function remove(id: string) {
        busy = true; failed = false;
        try { await api(token, '/api/admin/webhooks/' + encodeURIComponent(id), 'DELETE'); await load(); }
        catch { failed = true; } finally { busy = false; }
    }
</script>
<main class="mx-auto max-w-3xl px-4 py-8 text-white">
    <h1 class="mb-3 text-2xl">{t('notifications.title')}</h1>
    <p class="mb-6 text-sm text-neutral-400">{t('notifications.hint')}</p>
    <form onsubmit={create} class="mb-8 flex flex-col gap-4 rounded-lg bg-surface p-5">
        <label>{t('notifications.name')}<input class="mt-1 w-full rounded bg-base p-3" required maxlength="128" bind:value={name} /></label>
        <label>{t('notifications.url')}<input class="mt-1 w-full rounded bg-base p-3" type="url" required maxlength="2048" bind:value={url} /></label>
        <label>{t('notifications.destination')}<select class="mt-1 w-full rounded bg-base p-3" bind:value={destination}><option value="generic_json">JSON</option><option value="discord">Discord</option><option value="slack">Slack</option></select></label>
        <label class="flex gap-3"><input type="checkbox" bind:checked={enabled} />{t('notifications.enabled')}</label>
        <button disabled={busy} class="rounded bg-gold px-4 py-3 text-black disabled:opacity-50">{t('notifications.add')}</button>
    </form>
    {#each endpoints as endpoint}
        <div class="mb-3 flex items-center justify-between gap-3 rounded border border-white/10 p-4"><span>{endpoint.name} · {endpoint.destination_type} · {t(endpoint.enabled ? 'notifications.active' : 'notifications.inactive')}</span><button disabled={busy} class="rounded border border-white/20 px-3 py-2" onclick={() => remove(endpoint.id)}>{t('notifications.remove')}</button></div>
    {/each}
    <h2 class="mt-8 mb-3 text-xl">{t('notifications.deliveries')}</h2>
    <button class="mb-4 rounded border border-white/20 px-3 py-2" onclick={() => load().catch(() => (failed = true))}>{t('notifications.refresh')}</button>
    {#each deliveries as delivery}
        <p class="mb-2 rounded bg-surface p-3">{delivery.name} · {t('notifications.' + delivery.status)} · {t('notifications.attempts', { count: delivery.attempt_count })} {delivery.response_code ?? ''}</p>
    {/each}
    {#if failed}<p role="alert" class="mt-4 text-gold">{t('player.save_failed')}</p>{/if}
</main>
