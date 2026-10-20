// Production server for the web app: the SvelteKit build (adapter-node)
// plus a reverse proxy for `/api/*` — HTTP and WebSocket — to lisek-server,
// so browsers see one origin (cookies, no CORS) exactly as with `npm run dev`.
//
//   LISEK_API_URL   backend base URL (default http://127.0.0.1:8080)
//   PORT, HOST      where to listen (default 3000 on all interfaces)
//   TRUST_PROXY=1   this server sits behind another reverse proxy (Caddy,
//                   Traefik, nginx): keep its X-Forwarded-* headers instead
//                   of replacing them with this connection's address.
//
// `/healthz` answers 200 for container health checks.
import http from 'node:http';
import { handler } from './build/handler.js';

const API = new URL(process.env.LISEK_API_URL ?? 'http://127.0.0.1:8080');
const PORT = Number(process.env.PORT ?? 3000);
const HOST = process.env.HOST ?? '0.0.0.0';
const TRUST_PROXY = process.env.TRUST_PROXY === '1';

// Hop-by-hop headers (RFC 9110 §7.6.1) are not forwarded.
const HOP = new Set([
	'connection',
	'keep-alive',
	'proxy-connection',
	'proxy-authenticate',
	'proxy-authorization',
	'te',
	'trailer',
	'transfer-encoding',
	'upgrade'
]);

function isApi(url) {
	return url === '/api' || url.startsWith('/api/') || url.startsWith('/api?');
}

function forwardHeaders(req) {
	const headers = {};
	for (const [name, value] of Object.entries(req.headers)) {
		if (!HOP.has(name)) headers[name] = value;
	}
	const client = req.socket.remoteAddress ?? '';
	if (TRUST_PROXY && req.headers['x-forwarded-for']) {
		headers['x-forwarded-for'] = `${req.headers['x-forwarded-for']}, ${client}`;
	} else {
		// Never pass on a client-supplied address: the backend trusts this hop.
		headers['x-forwarded-for'] = client;
		headers['x-forwarded-proto'] = req.socket.encrypted ? 'https' : 'http';
		headers['x-forwarded-host'] = req.headers.host ?? '';
	}
	headers.host = API.host;
	return headers;
}

function responseHeaders(headers) {
	const out = {};
	for (const [name, value] of Object.entries(headers)) {
		if (!HOP.has(name) && value !== undefined) out[name] = value;
	}
	return out;
}

function upstreamOptions(req, headers) {
	return {
		protocol: API.protocol,
		hostname: API.hostname,
		port: API.port || (API.protocol === 'https:' ? 443 : 80),
		method: req.method,
		path: req.url,
		headers
	};
}

function proxy(req, res) {
	const upstream = http.request(upstreamOptions(req, forwardHeaders(req)), (answer) => {
		res.writeHead(answer.statusCode ?? 502, answer.statusMessage, responseHeaders(answer.headers));
		answer.pipe(res);
	});
	upstream.on('error', () => {
		if (!res.headersSent) {
			res.writeHead(502, { 'content-type': 'application/json' });
			res.end(JSON.stringify({ error_code: 'backend_unavailable' }));
		} else {
			res.destroy();
		}
	});
	// A viewer who closes the player stops the transfer (and the backend's work).
	res.on('close', () => {
		if (!res.writableFinished) upstream.destroy();
	});
	req.pipe(upstream);
}

function proxyUpgrade(req, socket, head) {
	const headers = forwardHeaders(req);
	headers.connection = 'Upgrade';
	headers.upgrade = req.headers.upgrade ?? 'websocket';
	const upstream = http.request(upstreamOptions(req, headers));
	upstream.on('upgrade', (answer, upstreamSocket, upstreamHead) => {
		const lines = [
			`HTTP/1.1 ${answer.statusCode ?? 101} ${answer.statusMessage ?? 'Switching Protocols'}`
		];
		for (let i = 0; i < answer.rawHeaders.length; i += 2) {
			lines.push(`${answer.rawHeaders[i]}: ${answer.rawHeaders[i + 1]}`);
		}
		socket.write(lines.join('\r\n') + '\r\n\r\n');
		if (upstreamHead?.length) socket.write(upstreamHead);
		if (head?.length) upstreamSocket.write(head);
		upstreamSocket.pipe(socket).pipe(upstreamSocket);
		upstreamSocket.on('error', () => socket.destroy());
		socket.on('error', () => upstreamSocket.destroy());
	});
	upstream.on('response', (answer) => {
		// The backend refused the upgrade (e.g. 401): pass the status on.
		socket.end(
			`HTTP/1.1 ${answer.statusCode} ${answer.statusMessage}\r\nconnection: close\r\n\r\n`
		);
	});
	upstream.on('error', () => socket.destroy());
	upstream.end();
}

const server = http.createServer((req, res) => {
	const url = req.url ?? '/';
	if (url === '/healthz') {
		res.writeHead(200, { 'content-type': 'text/plain' });
		res.end('ok');
		return;
	}
	if (isApi(url)) {
		proxy(req, res);
		return;
	}
	handler(req, res, () => {
		res.writeHead(404);
		res.end();
	});
});

server.on('upgrade', (req, socket, head) => {
	if (isApi(req.url ?? '')) proxyUpgrade(req, socket, head);
	else socket.destroy();
});

// HLS segments may take a while on a cold transcode; the backend has its
// own timeouts.
server.requestTimeout = 0;
server.headersTimeout = 60_000;
server.keepAliveTimeout = 65_000;

server.listen(PORT, HOST, () => {
	console.log(`LisekMovie web on http://${HOST}:${PORT} (API: ${API.origin})`);
});

for (const signal of ['SIGINT', 'SIGTERM']) {
	process.on(signal, () => {
		server.close(() => process.exit(0));
		setTimeout(() => process.exit(0), 5_000).unref();
	});
}
