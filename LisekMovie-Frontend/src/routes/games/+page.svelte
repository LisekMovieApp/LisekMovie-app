<script lang="ts">
    import { onMount } from 'svelte';
    import { t } from '$lib/i18n';
    import { getSessionCookieClientSide } from '$lib/auth';
    import { api } from '$lib/playback/api';

    let canvas = $state<HTMLCanvasElement>();
    let enabled = $state(false);
    let loading = $state(true);
    let failed = $state(false);
    let score = $state(0);
    let running = $state(false);
    let finished = $state(false);
    let ranking = $state<{ username: string; score: number }[]>([]);
    let token = '';
    let y = 220, velocity = 0, previous = 0, distance = 0;
    let pipes: { x: number; gap: number; counted: boolean }[] = [];

    async function loadRanking() {
        ranking = await api(token, '/api/games/flappy-bird/leaderboard');
    }
    function flap() {
        if (!enabled) return;
        if (!running) {
            score = 0;
            y = 220;
            velocity = 0;
            distance = 0;
            pipes = [];
            running = true;
            finished = false;
        }
        velocity = -270;
    }
    function finish() {
        running = false;
        finished = true;
        api(token, '/api/games/flappy-bird/scores', 'POST', { score })
            .then(loadRanking).catch(() => (failed = true));
    }
    onMount(() => {
        let active = true;
        token = getSessionCookieClientSide() ?? '';
        api<{ game_key: string }[]>(token, '/api/games').then(games => {
            if (!active) return;
            enabled = games.some(game => game.game_key === 'flappy-bird');
            if (enabled) return loadRanking();
        }).catch(() => { if (active) failed = true; })
            .finally(() => { if (active) loading = false; });
        let frame: number;
        function tick(time: number) {
            const dt = Math.min((time - previous) / 1000, 0.035);
            previous = time;
            const ctx = canvas?.getContext('2d');
            if (ctx) {
                ctx.fillStyle = '#17191e';
                ctx.fillRect(0, 0, 360, 480);
                if (running) {
                    velocity += 780 * dt;
                    y += velocity * dt;
                    distance += 125 * dt;
                    if (pipes.length === 0 || distance >= 200) {
                        pipes.push({ x: 360, gap: 100 + Math.random() * 180, counted: false });
                        distance = 0;
                    }
                    for (const pipe of pipes) {
                        pipe.x -= 125 * dt;
                        if (!pipe.counted && pipe.x + 46 < 76) { pipe.counted = true; score++; }
                        if (pipe.x < 88 && pipe.x + 46 > 64 && (y - 12 < pipe.gap || y + 12 > pipe.gap + 130)) finish();
                    }
                    pipes = pipes.filter(pipe => pipe.x > -50);
                    if (running && (y < 12 || y > 468)) finish();
                }
                ctx.fillStyle = '#bda24d';
                for (const pipe of pipes) {
                    ctx.fillRect(pipe.x, 0, 46, pipe.gap);
                    ctx.fillRect(pipe.x, pipe.gap + 130, 46, 480);
                }
                ctx.fillStyle = '#ed8d43';
                ctx.beginPath(); ctx.arc(76, y, 12, 0, Math.PI * 2); ctx.fill();
                ctx.fillStyle = '#fff'; ctx.beginPath(); ctx.arc(81, y - 3, 3, 0, Math.PI * 2); ctx.fill();
            }
            frame = requestAnimationFrame(tick);
        }
        frame = requestAnimationFrame(tick);
        const visibility = () => { if (document.hidden) running = false; };
        document.addEventListener('visibilitychange', visibility);
        return () => { active = false; cancelAnimationFrame(frame); document.removeEventListener('visibilitychange', visibility); };
    });
</script>
<main class="min-h-screen bg-base px-4 pt-28 pb-12 text-white">
    <div class="mx-auto max-w-3xl">
        <h1 class="mb-3 text-3xl font-semibold">{t('games.title')}</h1>
        {#if loading}<p>{t('common.loading')}</p>
        {:else if !enabled}<p>{t('games.disabled')}</p>
        {:else}
            <p class="mb-4 text-neutral-300">{t('games.instructions')}</p>
            <div class="flex flex-col gap-8 sm:flex-row">
                <div>
                    <p class="mb-3" aria-live="polite">{t('games.score', { score })}{finished ? ' · ' + t('games.finished') : ''}</p>
                    <button onclick={flap} class="block overflow-hidden rounded-xl border-2 border-gold focus-visible:outline-4 focus-visible:outline-white" aria-label={t('games.flap')}>
                        <canvas bind:this={canvas} width="360" height="480" class="max-w-full touch-manipulation"></canvas>
                    </button>
                </div>
                <section class="min-w-40"><h2 class="mb-3 text-xl">{t('games.ranking')}</h2>
                    <ol>{#each ranking as row}<li class="mb-2 flex justify-between gap-4"><span>{row.username}</span><span>{row.score}</span></li>{/each}</ol>
                </section>
            </div>
        {/if}
        {#if failed}<p role="alert" class="mt-4 text-gold">{t('player.load_failed')}</p>{/if}
    </div>
</main>
