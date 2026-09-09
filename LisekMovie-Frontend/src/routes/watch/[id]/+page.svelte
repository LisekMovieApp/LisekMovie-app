<script lang="ts">
    import Hls from 'hls.js';
    import { page } from '$app/state';
    import { resolve } from '$app/paths';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { startPlayback, saveProgress, type PlaybackSession } from '$lib/playback/api';

    let compatible = $state(false);
    let session = $state<PlaybackSession | null>(null);
    let video = $state<HTMLVideoElement>();
    let failed = $state(false);
    let saveFailed = $state(false);
    let loading = $state(true);
    let report = (_ended = false) => {};

    $effect(() => {
        const id = page.params.id ?? '';
        const useTranscode = compatible;
        const token = getSessionCookieClientSide();
        let active = true;
        let current: PlaybackSession | null = null;
        let queue = Promise.resolve();
        session = null;
        failed = false;
        loading = true;
        saveFailed = false;
        const send = (ended = false) => {
            if (!current || !video) return;
            const { currentTime, duration, paused } = video;
            const sessionId = current.session_id;
            queue = queue.catch(() => {}).then(() => saveProgress(token ?? '', sessionId, currentTime + (current?.time_offset_seconds ?? 0), current?.duration_seconds ?? duration, paused, ended))
                .then(() => { if (active) saveFailed = false; })
                .catch(() => { if (active) saveFailed = true; });
        };
        report = send;
        if (token) {
            startPlayback(token, id, useTranscode).then((result) => {
                if (!active) return;
                current = result;
                session = result;
            }).catch(() => { if (active) failed = true; })
                .finally(() => { if (active) loading = false; });
        } else {
            loading = false;
            failed = true;
        }
        const timer = setInterval(() => send(), 10000);
        const onPageHide = () => send();
        window.addEventListener('pagehide', onPageHide);
        return () => {
            send(true);
            active = false;
            clearInterval(timer);
            window.removeEventListener('pagehide', onPageHide);
        };
    });

    $effect(() => {
        if (!video || !session) return;
        const element = video;
        let hls: Hls | undefined;
        if (session.mode === 'transcode' && Hls.isSupported()) {
            hls = new Hls({ startPosition: 0 });
            hls.loadSource(session.stream_url);
            hls.attachMedia(element);
            hls.on(Hls.Events.ERROR, (_event, data) => { if (data.fatal) failed = true; });
        } else {
            element.src = session.stream_url;
        }
        return () => { hls?.destroy(); element.removeAttribute('src'); element.load(); };
    });

    function resume() {
        if (video && session && session.position_seconds > 0 && Number.isFinite(video.duration)) {
            video.currentTime = Math.min(session.position_seconds, Math.max(0, video.duration - 1));
        }
    }
</script>

<svelte:head><title>{session?.title ?? t('player.title')} · LisekMovie</title></svelte:head>
<main class="min-h-screen bg-base px-4 pt-28 pb-12 text-white md:px-8">
    <div class="mx-auto max-w-6xl">
        <a class="mb-6 inline-block rounded px-3 py-2 text-gold focus-visible:outline-2" href={resolve('/media/[id]', { id: page.params.id ?? '' })}>{t('common.back')}</a>
        <label class="mb-5 flex items-center gap-3 text-sm text-neutral-300"><input type="checkbox" bind:checked={compatible} />{t('player.compatible')}</label>
        {#if loading}
            <p role="status">{t('common.loading')}</p>
        {:else if failed}
            <p role="alert" class="rounded-lg bg-surface p-6">{t('player.failed')}</p>
        {:else if session}
            <h1 class="mb-4 text-2xl font-semibold">{session.title}</h1>
            <!-- Captions will be listed here when subtitle tracks are available. -->
            <!-- svelte-ignore a11y_media_has_caption -->
            <video bind:this={video} controls playsinline preload="metadata"
                class="aspect-video w-full rounded-lg bg-black"
                onloadedmetadata={resume} onplay={() => report()} onpause={() => report()}
                onended={() => report(true)} onerror={() => (failed = true)}>
            </video>
            {#if saveFailed}<p role="status" class="mt-3 text-gold">{t('player.save_failed')}</p>{/if}
        {/if}
    </div>
</main>
