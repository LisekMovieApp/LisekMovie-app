/**
 * Faza 20 — watching together. A room plays one film or episode for
 * everyone in it; play, pause and seek from any member reach every other
 * player (`/api/syncplay/socket`, authenticated with the session in the
 * first frame). The server keeps the room's state and sends it every
 * 10 seconds, so players that drifted apart are pulled back together.
 */
import { buildAuthHeader } from '$lib/auth';

export type SyncAction = 'play' | 'pause' | 'seek';

export interface RoomMember {
	user_id: string;
	username: string;
}

export interface RoomInfo {
	code: string;
	item_id: string;
	title: string;
	members: RoomMember[];
	playing: boolean;
	position: number;
}

export interface CreatedRoom {
	code: string;
	item_id: string;
	/** `/together/<code>`. */
	link: string;
}

export class RoomError extends Error {
	constructor(public code: string) {
		super(code);
	}
}

async function request<T>(token: string, path: string, init: RequestInit = {}): Promise<T> {
	const response = await fetch(path, {
		...init,
		cache: 'no-store',
		headers: { ...buildAuthHeader(token), 'Content-Type': 'application/json' }
	});
	const body = await response.json().catch(() => null);
	if (!response.ok) throw new RoomError(body?.error_code ?? 'internal_error');
	return body as T;
}

export function createRoom(token: string, itemId: string, position = 0): Promise<CreatedRoom> {
	return request(token, '/api/syncplay/rooms', {
		method: 'POST',
		body: JSON.stringify({ item_id: itemId, position })
	});
}

export function fetchRoom(token: string, code: string): Promise<RoomInfo> {
	return request(token, `/api/syncplay/rooms/${encodeURIComponent(code)}`);
}

/** A code as people type it: case, spaces and dashes do not matter. */
export function normalizeCode(value: string): string | null {
	const code = value.replace(/[\s-]/g, '').toUpperCase();
	return /^[A-HJ-NP-Z2-9]{6}$/.test(code) ? code : null;
}

/** The full link to share. */
export function roomLink(code: string, origin = location.origin): string {
	return `${origin}/together/${code}`;
}

export interface RoomHandlers {
	onjoined?: (room: RoomInfo) => void;
	onmembers?: (members: RoomMember[]) => void;
	oncommand?: (action: SyncAction, position: number, by: string) => void;
	onstate?: (playing: boolean, position: number) => void;
	/** `room_not_found`, `room_full` or `unauthorized`: no reconnecting. */
	onerror?: (code: string) => void;
	onconnection?: (connected: boolean) => void;
}

/** One member's connection to a room; reconnects on its own. */
export class SyncRoom {
	private socket: WebSocket | null = null;
	private retry = 1000;
	private timer: ReturnType<typeof setTimeout> | undefined;
	private closed = false;

	constructor(
		private code: string,
		private getToken: () => string | null,
		private handlers: RoomHandlers,
		private socketUrl = () => {
			const url = new URL('/api/syncplay/socket', location.href);
			url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
			return url.toString();
		}
	) {}

	connect() {
		if (this.closed) return;
		const token = this.getToken();
		if (!token) return this.handlers.onerror?.('unauthorized');
		const socket = new WebSocket(this.socketUrl());
		this.socket = socket;
		socket.onopen = () => socket.send(JSON.stringify({ token, code: this.code }));
		socket.onmessage = (event) => {
			if (this.socket !== socket) return;
			let data: Record<string, unknown>;
			try {
				data = JSON.parse(String(event.data));
			} catch {
				return;
			}
			this.receive(data);
		};
		socket.onclose = () => {
			if (this.socket !== socket) return;
			this.socket = null;
			this.handlers.onconnection?.(false);
			if (this.closed) return;
			this.timer = setTimeout(() => this.connect(), this.retry);
			this.retry = Math.min(this.retry * 2, 15000);
		};
		socket.onerror = () => socket.close();
	}

	private receive(data: Record<string, unknown>) {
		const position = typeof data.position === 'number' ? data.position : 0;
		switch (data.type) {
			case 'joined':
				this.retry = 1000;
				this.handlers.onconnection?.(true);
				this.handlers.onjoined?.(data as unknown as RoomInfo);
				break;
			case 'members':
				if (Array.isArray(data.members)) this.handlers.onmembers?.(data.members as RoomMember[]);
				break;
			case 'command':
				if (data.action === 'play' || data.action === 'pause' || data.action === 'seek')
					this.handlers.oncommand?.(data.action, position, String(data.by ?? ''));
				break;
			case 'state':
				this.handlers.onstate?.(data.playing === true, position);
				break;
			case 'error':
				this.closed = true;
				this.handlers.onerror?.(String(data.code ?? 'room_not_found'));
				break;
		}
	}

	send(action: SyncAction, position: number) {
		if (this.socket?.readyState !== WebSocket.OPEN || !Number.isFinite(position)) return;
		this.socket.send(JSON.stringify({ type: action, position: Math.max(0, position) }));
	}

	close() {
		this.closed = true;
		clearTimeout(this.timer);
		const socket = this.socket;
		this.socket = null;
		socket?.close();
	}
}

/** How far apart (seconds) a player may drift before it is corrected. */
export const DRIFT_TOLERANCE = 2.5;
