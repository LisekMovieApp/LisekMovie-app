// Plays an HLS session in headless Chromium with hls.js, the way the web
// player does: start, seek far ahead (the server restarts FFmpeg there),
// and show a subtitle rendition. Prints one JSON line with the results.
//
// Usage: node hls_browser_check.mjs '{"base":"http://127.0.0.1:8080","master":"/api/...","hlsjs":"/path/hls.min.js","seek":40}'
// Needs the `playwright` package (resolved through NODE_PATH, which ES
// module imports ignore; hence `require`).
import { createRequire } from 'node:module';

const { chromium } = createRequire(import.meta.url)('playwright');

const options = JSON.parse(process.argv[2]);
const browser = await chromium.launch();
try {
    const page = await browser.newPage();
    // Same origin as the API, so no CORS setup is needed.
    await page.goto(options.base + '/api/health');
    await page.addScriptTag({ path: options.hlsjs });
    const result = await page.evaluate(async ({ master, seek }) => {
        const until = (check, ms, what) =>
            new Promise((resolve, reject) => {
                const started = Date.now();
                const timer = setInterval(() => {
                    if (check()) { clearInterval(timer); resolve(); }
                    else if (Date.now() - started > ms) { clearInterval(timer); reject(new Error(what)); }
                }, 100);
            });
        const video = document.createElement('video');
        video.muted = true;
        document.body.append(video);
        const errors = [];
        const hls = new Hls({ maxBufferLength: 30 });
        hls.on(Hls.Events.ERROR, (_e, data) => { if (data.fatal) errors.push(data.details); });
        hls.loadSource(master);
        hls.attachMedia(video);
        await until(() => hls.levels.length > 0, 20000, 'manifest');
        await video.play();
        await until(() => video.currentTime > 2, 30000, 'playback start');
        const duration = video.duration;
        const before = performance.now();
        video.currentTime = seek;
        await until(() => video.currentTime > seek + 1, 40000, 'playback after seek');
        const seekMs = performance.now() - before;
        hls.subtitleDisplay = true;
        hls.subtitleTrack = 0;
        const tracks = hls.subtitleTracks.map((t) => t.name);
        await until(() => [...video.textTracks].some((t) => t.cues && t.cues.length > 0), 20000, 'subtitle cues');
        const cues = [...video.textTracks].flatMap((t) => [...(t.cues ?? [])].map((c) => c.text));
        return { duration, seekMs, tracks, cues, errors, position: video.currentTime };
    }, { master: options.master, seek: options.seek ?? 40 });
    console.log(JSON.stringify({ ok: true, ...result }));
} catch (error) {
    console.log(JSON.stringify({ ok: false, error: String(error) }));
} finally {
    await browser.close();
}
