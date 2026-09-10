// Integration probe; credentials enter through stdin, never through command-line arguments.
import { createInterface } from 'node:readline';
const input = createInterface({ input: process.stdin });
input.once('line', line => {
    const { base, token } = JSON.parse(line);
    const ws = new WebSocket(base.replace('http:', 'ws:') + '/api/notifications/socket');
    const timeout = setTimeout(() => { ws.close(); process.exit(2); }, 180000);
    ws.onopen = () => ws.send(JSON.stringify({ token }));
    ws.onmessage = event => process.stdout.write(event.data + '\n');
    ws.onclose = () => { clearTimeout(timeout); process.stdout.write('{"type":"closed"}\n'); input.close(); process.exit(0); };
    ws.onerror = () => { clearTimeout(timeout); process.exit(1); };
});
