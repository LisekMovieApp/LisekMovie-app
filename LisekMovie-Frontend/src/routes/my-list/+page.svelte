<script lang="ts">
    import { onMount } from 'svelte';
    import { resolve } from '$app/paths';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { api, type ListEntry } from '$lib/playback/api';
    import { imageUrl } from '$lib/media/api';
    let items = $state<ListEntry[]>([]);
    let token = $state('');
    let loading = $state(true);
    let failed = $state(false);
    let filter = $state('continue');
    const visible = $derived(items.filter(item => filter === 'favorites' ? item.favorite : filter === 'watched' ? item.watched : item.position_seconds > 0 && !item.watched));
    onMount(async () => {
        token = getSessionCookieClientSide() ?? '';
        try { items = await api<ListEntry[]>(token, '/api/me/list'); }
        catch { failed = true; }
        finally { loading = false; }
    });
</script>
<main class="min-h-screen bg-base px-4 pt-28 pb-12 text-white md:px-8">
    <h1 class="mb-6 text-3xl font-semibold">{t('nav.my_list')}</h1>
    <div class="mb-6 flex flex-wrap gap-3">
        {#each ['continue', 'favorites', 'watched'] as key}
            <button class="rounded-md border px-4 py-2 {filter === key ? 'border-gold text-gold' : 'border-white/20'}" aria-pressed={filter === key} onclick={() => (filter = key)}>{t('player.list_' + key)}</button>
        {/each}
    </div>
    {#if loading}<p role="status">{t('common.loading')}</p>
    {:else if failed}<p role="alert">{t('player.load_failed')}</p>
    {:else if visible.length === 0}<p class="text-neutral-400">{t('player.list_empty')}</p>
    {:else}
        <div class="grid grid-cols-2 gap-5 sm:grid-cols-3 lg:grid-cols-6">
            {#each visible as item (item.id)}
                <a href={resolve('/media/[id]', { id: item.id })} class="overflow-hidden rounded-lg bg-surface focus-visible:outline-2 focus-visible:outline-gold">
                    <img class="aspect-[2/3] w-full object-cover" src={imageUrl('/api/media/' + item.id + '/poster', token)} alt="" loading="lazy" />
                    <div class="p-3"><h2 class="font-medium">{item.title}</h2>
                        {#if item.duration_seconds > 0}<progress class="mt-2 w-full accent-gold" max={item.duration_seconds} value={item.position_seconds} aria-label={t('player.progress')}></progress>{/if}
                    </div>
                </a>
            {/each}
        </div>
    {/if}
</main>
