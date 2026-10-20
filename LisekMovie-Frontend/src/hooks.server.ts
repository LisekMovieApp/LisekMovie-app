// Server-side `fetch('/api/...')` in load functions (`+layout.ts` asks
// /api/status and /api/auth/me while rendering on the server).
//
// In production the web app (adapter-node, `server.js`) and lisek-server
// are separate processes or containers: a relative `/api` URL would come
// back to this Node server, which renders pages, not API responses. With
// `LISEK_API_URL` set (docker-compose sets `http://lisek-server:8080`)
// such requests go straight to the backend instead. Unset (`npm run dev`)
// nothing changes: Vite's proxy serves them.
import type { HandleFetch } from '@sveltejs/kit';
import { env } from '$env/dynamic/private';

export const handleFetch: HandleFetch = async ({ event, request, fetch }) => {
	const api = env.LISEK_API_URL;
	const url = new URL(request.url);
	if (api && url.origin === event.url.origin && url.pathname.startsWith('/api/')) {
		const target = new URL(url.pathname + url.search, api);
		const forwarded = new Request(target, request);
		// The backend decides LAN vs. remote limits by the client address.
		forwarded.headers.set('x-forwarded-for', event.getClientAddress());
		return fetch(forwarded);
	}
	return fetch(request);
};
