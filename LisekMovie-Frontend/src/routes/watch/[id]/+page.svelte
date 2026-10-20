<script lang="ts">
    import Hls from 'hls.js';
    import { page } from '$app/state';
    import { resolve } from '$app/paths';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import {
        startPlayback,
        saveProgress,
        stopPlayback,
        type DownmixPreference,
        type PlaybackSession,
        type StartRequest
    } from '$lib/playback/api';
    import { detectCapabilities, type ClientCapabilities } from '$lib/playback/capabilities';
    import { audioLabel, subtitleChange, subtitleLabel } from '$lib/playback/tracks';

    const DOWNMIX: DownmixPreference[] = ['auto', 'standard', 'dialogue', 'night', 'pro_logic_ii'];

    let session = $state<PlaybackSession | null>(null);
    let video = $state<HTMLVideoElement>();
    let failed = $state(false);
    let saveFailed = $state(false);
    let loading = $state(true);
    // The viewer's choices. Changing any but the subtitle starts a new
    // session at the current position; subtitles switch in place when the
    // running session carries them.
    let audio = $state<number | null>(null);
    let subtitle = $state('');
    let burnIn = $state(false);
    let downmix = $state<DownmixPreference>('auto');
    let compatible = $state(false);
    let hls: Hls | undefined;
    let caps: ClientCapabilities | undefined;
    let token = '';
    let current: PlaybackSession | null = null;
    let queue = Promise.resolve();
    let opened = 0;

    function report(ended = false, target = current) {
        if (!target) return;
        const { currentTime = 0, duration = 0, paused = true } = video ?? {};
        const total = target.duration_seconds ?? duration;
        const position = currentTime + target.time_offset_seconds;
        const sessionId = target.session_id;
        queue = queue
            .catch(() => {})
            .then(async () => {
                try { await saveProgress(token, sessionId, position, total, paused, ended); }
                finally { if (ended) await stopPlayback(token, sessionId); }
            })
            .then(() => { saveFailed = false; })
            .catch(() => { saveFailed = true; });
    }

    async function open(position?: number) {
        const attempt = ++opened;
        const previous = current;
        const request: StartRequest = {
            client: caps,
            audio_stream_index: audio,
            subtitle: subtitle || null,
            burn_subtitle: burnIn,
            force_transcode: compatible,
            downmix,
            start_position_seconds: position ?? null
        };
        loading = previous === null;
        failed = false;
        try {
            const next = await startPlayback(token, page.params.id ?? '', request);
            if (attempt !== opened) { void stopPlayback(token, next.session_id).catch(() => {}); return; }
            if (previous) report(true, previous);
            current = next;
            session = next;
            audio = next.selected_audio_stream_index ?? null;
            subtitle = next.selected_subtitle ?? '';
        } catch {
            if (attempt === opened) failed = true;
        } finally {
            if (attempt === opened) loading = false;
        }
    }

    function restart() {
        void open(video?.currentTime ?? current?.position_seconds);
    }

    function applySubtitle() {
        const track = session?.subtitle_tracks.find((s) => s.id === subtitle);
        const rendition = track?.hls_rendition ?? -1;
        if (hls) {
            hls.subtitleDisplay = true;
            hls.subtitleTrack = rendition;
        } else if (video && session?.hls) {
            // Native HLS (Safari): one text track per rendition, in order.
            Array.from(video.textTracks).forEach((tt, i) => { tt.mode = i === rendition ? 'showing' : 'disabled'; });
        }
    }

    function onSubtitle() {
        if (session && subtitleChange(session, subtitle, burnIn) === 'switch') applySubtitle();
        else restart();
    }

    $effect(() => {
        void page.params.id;
        token = getSessionCookieClientSide() ?? '';
        current = null;
        session = null;
        if (!token) { loading = false; failed = true; return; }
        caps = detectCapabilities();
        void open();
        const timer = setInterval(() => report(), 10000);
        const onPageHide = () => report(true);
        window.addEventListener('pagehide', onPageHide);
        return () => {
            report(true);
            opened++;
            current = null;
            clearInterval(timer);
            window.removeEventListener('pagehide', onPageHide);
        };
    });

    $effect(() => {
        if (!video || !session) return;
        const element = video;
        const playing = session;
        if (playing.hls && Hls.isSupported()) {
            hls = new Hls({ startPosition: playing.position_seconds, maxBufferLength: 30 });
            hls.on(Hls.Events.SUBTITLE_TRACKS_UPDATED, applySubtitle);
            hls.on(Hls.Events.ERROR, (_event, data) => { if (data.fatal) failed = true; });
            hls.loadSource(playing.stream_url);
            hls.attachMedia(element);
        } else {
            element.src = playing.stream_url;
        }
        return () => {
            hls?.destroy();
            hls = undefined;
            element.removeAttribute('src');
            element.load();
        };
    });

    function resume() {
        // hls.js starts at `startPosition` itself.
        if (!hls && video && session && session.position_seconds > 0 && Number.isFinite(video.duration)) {
            video.currentTime = Math.min(session.position_seconds, Math.max(0, video.duration - 1));
        }
        applySubtitle();
    }

    const directSubtitle = $derived(
        session && !session.hls ? session.subtitle_tracks.find((s) => s.id === subtitle && s.url) : undefined
    );
</script>

<svelte:head><title>{session?.title ?? t('player.title')} · LisekMovie</title></svelte:head>
<main class="min-h-screen bg-base px-4 pt-28 pb-12 text-white md:px-8">
    <div class="mx-auto max-w-6xl">
        <a class="mb-6 inline-block rounded px-3 py-2 text-gold focus-visible:outline-2" href={resolve('/media/[id]', { id: page.params.id ?? '' })}>{t('common.back')}</a>
        {#if loading}
            <p role="status">{t('common.loading')}</p>
        {:else if failed && !session}
            <p role="alert" class="rounded-lg bg-surface p-6">{t('player.failed')}</p>
        {:else if session}
            <h1 class="mb-4 text-2xl font-semibold">{session.title}</h1>
            <video bind:this={video} controls playsinline preload="metadata" crossorigin="anonymous"
                class="aspect-video w-full rounded-lg bg-black"
                onloadedmetadata={resume} onplay={() => report()} onpause={() => report()}
                onended={() => report()} onerror={() => (failed = true)}>
                {#if directSubtitle?.url}
                    <track kind="subtitles" src={directSubtitle.url} srclang={directSubtitle.language ?? undefined}
                        label={subtitleLabel(directSubtitle, t('player.forced'))} default />
                {/if}
            </video>
            {#if failed}<p role="alert" class="mt-3 text-gold">{t('player.failed')}</p>{/if}
            <div class="mt-4 grid gap-4 text-sm md:grid-cols-3">
                <label class="flex flex-col gap-1">{t('player.audio_track')}
                    <select bind:value={audio} onchange={restart}>
                        {#each session.audio_tracks as track (track.index)}
                            <option value={track.index}>{audioLabel(track)}</option>
                        {/each}
                    </select>
                </label>
                <label class="flex flex-col gap-1">{t('player.subtitles')}
                    <select bind:value={subtitle} onchange={onSubtitle}>
                        <option value="">{t('player.subtitles_off')}</option>
                        {#each session.subtitle_tracks as track (track.id)}
                            <option value={track.id}>{subtitleLabel(track, t('player.forced'))}{track.kind === 'image' ? ` (${t('player.burnt_in')})` : ''}</option>
                        {/each}
                    </select>
                </label>
                <label class="flex flex-col gap-1">{t('player.downmix')}
                    <select bind:value={downmix} onchange={restart}>
                        {#each DOWNMIX as mode (mode)}<option value={mode}>{t(`player.downmix_modes.${mode}`)}</option>{/each}
                    </select>
                </label>
                <label class="flex items-center gap-3"><input type="checkbox" bind:checked={burnIn} onchange={() => { if (subtitle) restart(); }} />{t('player.burn_in')}</label>
                <label class="flex items-center gap-3"><input type="checkbox" bind:checked={compatible} onchange={restart} />{t('player.compatible')}</label>
            </div>
            <p class="mt-4 text-xs text-neutral-400" role="status">
                {t(`player.modes.${session.mode}`)} ·
                {session.delivered.video_codec.toUpperCase()} {session.delivered.width}×{session.delivered.height}
                {#if session.delivered.tone_mapped}· {t('player.tone_mapped')}{/if}
                {#if session.delivered.audio_codec}· {session.delivered.audio_codec.toUpperCase()} {t('player.channels', { count: session.delivered.audio_channels ?? 2 })}{/if}
                {#if session.delivered.downmix}· {t(`player.downmix_modes.${session.delivered.downmix}`)}{/if}
            </p>
            {#if session.reasons.length}
                <p class="mt-1 text-xs text-neutral-500">{t('player.reasons_label')} {session.reasons.map((r) => t(`player.reasons.${r}`)).join(', ')}</p>
            {/if}
            {#if saveFailed}<p role="status" class="mt-3 text-gold">{t('player.save_failed')}</p>{/if}
        {/if}
    </div>
</main>

<style>
    select { padding: 0.55rem; background: #252525; border: 1px solid #ffffff20; border-radius: 0.4rem; color: white; }
    select:focus-visible, input:focus-visible { outline: 2px solid #d5ad42; outline-offset: 2px; }
</style>
