<script lang="ts">
    import { onMount } from 'svelte';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { page } from '$app/state';
    import icon from '$lib/assets/favicon.svg';
    import Icon from './Icon.svelte';
    let { userId }: { userId: string } = $props();
    let enabled = $state(false), supported = $state(false), connected = $state(false), busy = $state(false);
    let message = $state('');
    let toggle = () => {};
    onMount(() => {
        supported = window.isSecureContext && 'Notification' in window;
        const key = `lisek-notifications:${userId}`;
        let active = true;
        let socket: WebSocket | undefined;
        let timer: ReturnType<typeof setTimeout> | undefined;
        let retry = 1000;
        let registration: ServiceWorkerRegistration | undefined;
        // eslint-disable-next-line svelte/prefer-svelte-reactivity -- plain dedup cache, not UI state
        const seen = new Set<string>();
        try { enabled = supported && localStorage.getItem(key) === 'true' && Notification.permission === 'granted'; } catch { /* Storage may be unavailable. */ }

        function stop() {
            clearTimeout(timer); socket?.close(); socket = undefined; connected = false;
        }
        // The app's service worker (`src/service-worker.ts`, registered by
        // SvelteKit) shows the notifications and handles their clicks.
        // Without one (plain HTTP, dev browsers) desktop browsers fall back
        // to nonpersistent notifications.
        async function register() {
            if ('serviceWorker' in navigator) {
                try {
                    registration = await Promise.race([
                        navigator.serviceWorker.ready,
                        new Promise<undefined>(resolve => setTimeout(() => resolve(undefined), 4000))
                    ]);
                } catch { /* Desktop browsers can use nonpersistent notifications. */ }
            }
        }
        async function show(item: { item_id: string; title: string }) {
            if (!active || !enabled || Notification.permission !== 'granted') return;
            const url = `/media/${encodeURIComponent(item.item_id)}`;
            const options = { body: item.title, icon, tag: `lisek-item-${item.item_id}`, data: { url } };
            const title = t('browser_notifications.new_in_your_library');
            try {
                if (registration) await registration.showNotification(title, options);
                else {
                    const notification = new Notification(title, options);
                    notification.onclick = () => { window.focus(); window.location.assign(url); notification.close(); };
                    setTimeout(() => notification.close(), 15000);
                }
            } catch { message = 'browser_notifications.display_failed'; }
        }
        function connect() {
            if (!active || !enabled || !getSessionCookieClientSide()) return;
            // eslint-disable-next-line svelte/prefer-svelte-reactivity -- one-off socket address
            const url = new URL('/api/notifications/socket', window.location.href);
            url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
            socket = new WebSocket(url);
            const current = socket;
            current.onopen = () => current.send(JSON.stringify({ token: getSessionCookieClientSide() }));
            current.onmessage = event => {
                if (!active || !enabled || socket !== current) return;
                try {
                    const data = JSON.parse(event.data);
                    if (data.type === 'ready') { connected = true; retry = 1000; return; }
                    if (data.type !== 'library_changed' || !Array.isArray(data.items)) return;
                    for (const item of data.items.slice(0, 3)) {
                        if (typeof item.item_id !== 'string' || !/^[0-9a-f-]{36}$/i.test(item.item_id) || typeof item.title !== 'string' || seen.has(item.item_id)) continue;
                        seen.add(item.item_id);
                        if (seen.size > 1000) seen.delete(seen.values().next().value!);
                        // Match Jellyfin's behavior: do not interrupt video playback.
                        if (!page.url.pathname.startsWith('/watch/')) void show(item);
                    }
                } catch { /* Ignore malformed server frames. */ }
            };
            current.onclose = () => {
                if (socket !== current) return;
                connected = false;
                if (active && enabled) {
                    timer = setTimeout(connect, retry);
                    retry = Math.min(retry * 2, 30000);
                }
            };
            current.onerror = () => current.close();
        }
        toggle = async () => {
            if (busy) return;
            busy = true; message = '';
            try {
                if (enabled) { enabled = false; stop(); }
                else if (supported) {
                    const permission = await Notification.requestPermission();
                    if (!active) return;
                    if (permission === 'granted') { enabled = true; await register(); if (active) connect(); }
                    else message = 'browser_notifications.permission_not_granted';
                }
                try { localStorage.setItem(key, String(enabled)); } catch { /* Preference is session-only. */ }
            } finally { if (active) busy = false; }
        };
        if (enabled) void register().then(() => { if (active) connect(); });
        return () => {
            active = false; stop();
            void registration?.getNotifications().then(items => items.filter(n => n.tag.startsWith('lisek-item-')).forEach(n => n.close()));
        };
    });
</script>
<!-- Faza 19: a row of the account menu. The state comes from this device
     (the saved choice and the browser's permission), so the row offers the
     opposite action: "Notifications: Turn off" while they are on. -->
<div class="text-sm">
    <button type="button" class="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-neutral-200 transition-colors hover:bg-white/10 hover:text-white focus-visible:bg-white/10 disabled:cursor-not-allowed disabled:opacity-50" disabled={!supported || busy} onclick={() => toggle()} aria-pressed={enabled}>
        <Icon name={enabled ? 'bell' : 'bell_off'} class="size-4.5 shrink-0 {enabled ? 'text-gold' : 'text-neutral-400'}" />
        <span class="flex-1">{enabled ? t('browser_notifications.menu_disable') : t('browser_notifications.menu_enable')}</span>
        <span class="rounded-full px-2 py-0.5 text-[0.65rem] font-semibold tracking-wide uppercase {enabled ? 'bg-gold/15 text-gold' : 'bg-white/5 text-neutral-400'}">{enabled ? t('browser_notifications.state_on') : t('browser_notifications.state_off')}</span>
    </button>
    <p class="px-3 pb-1 pl-10.5 text-xs text-neutral-500">{!supported ? t('browser_notifications.unsupported_hint') : enabled ? connected ? t('browser_notifications.connected_new_titles') : t('browser_notifications.connecting_to_server') : t('browser_notifications.enable_hint')}</p>
    {#if message}<p role="status" class="px-3 pl-10.5 text-xs text-gold">{t(message)}</p>{/if}
</div>
