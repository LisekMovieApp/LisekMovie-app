import { afterEach, describe, expect, it, vi } from 'vitest';
import { normalizeCode, SyncRoom, type RoomHandlers } from './room';

class FakeSocket {
	static OPEN = 1;
	static last: FakeSocket | null = null;
	readyState = 0;
	sent: string[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((event: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;
	constructor(public url: string) {
		FakeSocket.last = this;
	}
	send(data: string) {
		this.sent.push(data);
	}
	close() {
		this.readyState = 3;
		this.onclose?.();
	}
	open() {
		this.readyState = FakeSocket.OPEN;
		this.onopen?.();
	}
	receive(data: unknown) {
		this.onmessage?.({ data: JSON.stringify(data) });
	}
}

afterEach(() => {
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

function connect(handlers: RoomHandlers) {
	vi.stubGlobal('WebSocket', FakeSocket);
	const room = new SyncRoom(
		'ABC234',
		() => 'tok',
		handlers,
		() => 'ws://test/api/syncplay/socket'
	);
	room.connect();
	const socket = FakeSocket.last!;
	socket.open();
	return { room, socket };
}

describe('watch-together rooms', () => {
	it('normalizes typed codes', () => {
		expect(normalizeCode(' abc-234 ')).toBe('ABC234');
		expect(normalizeCode('ABC23')).toBeNull();
		expect(normalizeCode('ABCDE0')).toBeNull();
	});

	it('authenticates in the first frame and relays room events', () => {
		const handlers = {
			onjoined: vi.fn(),
			onmembers: vi.fn(),
			oncommand: vi.fn(),
			onstate: vi.fn(),
			onconnection: vi.fn()
		};
		const { room, socket } = connect(handlers);
		expect(JSON.parse(socket.sent[0])).toEqual({ token: 'tok', code: 'ABC234' });
		socket.receive({
			type: 'joined',
			code: 'ABC234',
			item_id: 'i',
			members: [],
			playing: true,
			position: 12
		});
		expect(handlers.onjoined).toHaveBeenCalledWith(
			expect.objectContaining({ playing: true, position: 12 })
		);
		expect(handlers.onconnection).toHaveBeenCalledWith(true);
		socket.receive({ type: 'command', action: 'pause', position: 40.5, by: 'Anna' });
		expect(handlers.oncommand).toHaveBeenCalledWith('pause', 40.5, 'Anna');
		socket.receive({ type: 'command', action: 'explode', position: 1 });
		expect(handlers.oncommand).toHaveBeenCalledTimes(1);
		socket.receive({ type: 'state', playing: false, position: 41 });
		expect(handlers.onstate).toHaveBeenCalledWith(false, 41);
		room.send('seek', 100);
		room.send('seek', Number.NaN);
		expect(socket.sent.slice(1).map((s) => JSON.parse(s))).toEqual([
			{ type: 'seek', position: 100 }
		]);
		room.close();
	});

	it('reconnects after a drop, but not after a refusal', () => {
		vi.useFakeTimers();
		const onerror = vi.fn();
		const { socket } = connect({ onerror });
		socket.close();
		vi.advanceTimersByTime(1000);
		const second = FakeSocket.last!;
		expect(second).not.toBe(socket);
		second.open();
		second.receive({ type: 'error', code: 'room_full' });
		expect(onerror).toHaveBeenCalledWith('room_full');
		second.close();
		vi.advanceTimersByTime(30_000);
		expect(FakeSocket.last).toBe(second);
	});
});
