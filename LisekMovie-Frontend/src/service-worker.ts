/// <reference no-default-lib="true"/>
/// <reference lib="esnext" />
/// <reference lib="webworker" />
/// <reference types="@sveltejs/kit" />

// LisekMovie's one service worker (Faza 6, PWA). SvelteKit builds and
// registers it at `/service-worker.js`, scope `/`.
//
// - App shell: the built JS/CSS chunks and the static files (icons,
//   manifest, offline page) are cached at install, per deployed version,
//   and served cache-first — the app opens fast and a new version replaces
//   the whole set at once.
// - Pages: always from the network (they carry the signed-in account's
//   data). Without a connection the offline page is shown instead.
// - `/api/*` is never touched: authentication headers, video streams with
//   Range requests, HLS segments and WebSocket upgrades go straight to the
//   server, exactly as without a service worker.
// - Notifications: a click on a "new in your library" notification opens
//   that title. This replaces the older `notifications-worker.js` (same
//   scope); that file stays in `static/` for browsers still running it
//   until this worker takes over their registration.

import { build, files, version } from '$service-worker';

const sw = self as unknown as ServiceWorkerGlobalScope;
const CACHE = `lisek-shell-${version}`;
const OFFLINE = '/offline.html';
const SHELL = [...build, ...files.filter((file) => file !== '/notifications-worker.js')];
const SHELL_SET = new Set(SHELL);

sw.addEventListener('install', (event) => {
	event.waitUntil(
		caches
			.open(CACHE)
			.then((cache) => cache.addAll(SHELL))
			.then(() => sw.skipWaiting())
	);
});

sw.addEventListener('activate', (event) => {
	event.waitUntil(
		(async () => {
			for (const key of await caches.keys()) {
				if (key !== CACHE && key.startsWith('lisek-shell-')) await caches.delete(key);
			}
			await sw.clients.claim();
		})()
	);
});

sw.addEventListener('fetch', (event) => {
	const request = event.request;
	if (request.method !== 'GET') return;
	const url = new URL(request.url);
	if (url.origin !== sw.location.origin || url.pathname.startsWith('/api/')) return;

	if (SHELL_SET.has(url.pathname)) {
		event.respondWith(
			caches.match(url.pathname).then((cached) => cached ?? fetch(request))
		);
		return;
	}

	if (request.mode === 'navigate') {
		event.respondWith(
			fetch(request).catch(async () => (await caches.match(OFFLINE)) ?? Response.error())
		);
	}
});

const MEDIA_PATH = /^\/media\/[0-9a-f-]{36}$/i;

sw.addEventListener('notificationclick', (event) => {
	event.notification.close();
	const path = event.notification.data?.url;
	if (typeof path !== 'string' || !MEDIA_PATH.test(path)) return;
	const target = new URL(path, sw.location.origin).href;
	event.waitUntil(
		(async () => {
			const windows = await sw.clients.matchAll({ type: 'window', includeUncontrolled: true });
			const client = windows.find((w) => new URL(w.url).origin === sw.location.origin);
			if (client) {
				await client.navigate(target);
				await client.focus();
			} else {
				await sw.clients.openWindow(target);
			}
		})()
	);
});
