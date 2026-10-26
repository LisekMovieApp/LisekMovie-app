// Downloads (Faza 7): the original file of a film or an episode, for
// watching offline. The server hands out a short-lived URL (a browser
// download cannot send the session header); opening it starts the
// download with a readable file name.

import type { components } from '$lib/api/schema';
import { api } from '$lib/playback/api';

export type MyAccess = components['schemas']['MyAccessResponse'];
export type DownloadTicket = components['schemas']['DownloadTicketResponse'];

export function fetchMyAccess(token: string): Promise<MyAccess> {
	return api<MyAccess>(token, '/api/me/access');
}

export function downloadTicket(token: string, itemId: string): Promise<DownloadTicket> {
	return api<DownloadTicket>(
		token,
		`/api/media/${encodeURIComponent(itemId)}/download-ticket`,
		'POST',
		undefined,
		false
	);
}

/** Starts the browser's own download of `ticket` (no page change). */
export function startDownload(ticket: DownloadTicket, doc: Document = document): void {
	const link = doc.createElement('a');
	link.href = ticket.url;
	link.download = ticket.file_name;
	link.rel = 'noopener';
	doc.body.appendChild(link);
	link.click();
	link.remove();
}

/** "1.4 GB", "700 MB". */
export function formatSize(bytes: number): string {
	const units = ['B', 'KB', 'MB', 'GB', 'TB'];
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit += 1;
	}
	return `${value >= 10 || unit === 0 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}
