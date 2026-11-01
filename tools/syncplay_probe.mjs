// Integration probe for watching together (Faza 20). Credentials enter
// through stdin, never through command-line arguments. Two members join one
// room: A seeks and plays, B pauses; each must hear the other.
import { createInterface } from 'node:readline';

const input = createInterface({ input: process.stdin });

function member(url, token, code) {
	const ws = new WebSocket(url);
	const queue = [];
	const waiting = [];
	ws.onopen = () => ws.send(JSON.stringify({ token, code }));
	ws.onmessage = (event) => {
		const data = JSON.parse(event.data);
		const index = waiting.findIndex((w) => w.match(data));
		if (index >= 0) waiting.splice(index, 1)[0].resolve(data);
		else queue.push(data);
	};
	return {
		ws,
		send: (data) => ws.send(JSON.stringify(data)),
		next: (match) =>
			new Promise((resolve, reject) => {
				const index = queue.findIndex(match);
				if (index >= 0) return resolve(queue.splice(index, 1)[0]);
				const timer = setTimeout(() => reject(new Error('timeout')), 10000);
				waiting.push({ match, resolve: (d) => { clearTimeout(timer); resolve(d); } });
			})
	};
}

input.once('line', async (line) => {
	const { base, a: tokenA, b: tokenB, code } = JSON.parse(line);
	const url = base.replace('http:', 'ws:') + '/api/syncplay/socket';
	try {
		const a = member(url, tokenA, code);
		await a.next((d) => d.type === 'joined');
		const b = member(url, tokenB, code);
		const joinedB = await b.next((d) => d.type === 'joined');
		await a.next((d) => d.type === 'members' && d.members.length === 2);

		a.send({ type: 'seek', position: 120 });
		const bGot = await b.next((d) => d.type === 'command');
		a.send({ type: 'play', position: 120 });
		await b.next((d) => d.type === 'command' && d.action === 'play');
		await new Promise((r) => setTimeout(r, 300));
		b.send({ type: 'pause', position: 125 });
		const aGot = await a.next((d) => d.type === 'command');
		a.send({ type: 'state' });
		const state = await a.next((d) => d.type === 'state');

		const bad = member(url, tokenA, 'ZZZZZZ');
		const badCode = await bad.next((d) => d.type === 'error');
		for (const m of [a, b, bad]) m.ws.close();
		process.stdout.write(
			JSON.stringify({ joined_b: joinedB, b_got: bGot, a_got: aGot, state, bad_code: badCode }) + '\n'
		);
		process.exit(0);
	} catch (error) {
		process.stderr.write(String(error) + '\n');
		process.exit(1);
	}
});
