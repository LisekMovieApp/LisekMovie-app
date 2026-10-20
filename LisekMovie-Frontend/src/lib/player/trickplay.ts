/**
 * Seek-bar previews from the server's trickplay sprite sheets: which tile
 * of which sheet shows a given moment, and the sheet URLs (images, so the
 * token travels as `?token=` like posters do).
 */
import type { components } from '$lib/api/schema';
import { api, ApiError } from '$lib/playback/api';

export type TrickplayManifest = components['schemas']['TrickplayManifest'];

/** One tile: draw `url` as a background, offset by `-x, -y`. */
export interface Thumbnail {
	url: string;
	x: number;
	y: number;
	width: number;
	height: number;
	sheetWidth: number;
	sheetHeight: number;
}

export function withToken(url: string, token: string): string {
	return `${url}${url.includes('?') ? '&' : '?'}token=${encodeURIComponent(token)}`;
}

/** The tile for `time` seconds, or null outside the manifest. */
export function thumbnailAt(
	manifest: TrickplayManifest,
	time: number,
	token: string
): Thumbnail | null {
	if (!Number.isFinite(time) || time < 0 || manifest.count <= 0 || manifest.interval_seconds <= 0) {
		return null;
	}
	const index = Math.min(manifest.count - 1, Math.floor(time / manifest.interval_seconds));
	const perSheet = manifest.columns * manifest.rows;
	const sheet = Math.floor(index / perSheet);
	const url = manifest.sheets[sheet];
	if (!url) return null;
	const cell = index % perSheet;
	return {
		url: withToken(url, token),
		x: (cell % manifest.columns) * manifest.width,
		y: Math.floor(cell / manifest.columns) * manifest.height,
		width: manifest.width,
		height: manifest.height,
		sheetWidth: manifest.columns * manifest.width,
		sheetHeight: manifest.rows * manifest.height
	};
}

/** The manifest, or null while the server has not made the thumbnails yet. */
export async function fetchTrickplay(token: string, id: string): Promise<TrickplayManifest | null> {
	try {
		return await api<TrickplayManifest>(token, `/api/media/${encodeURIComponent(id)}/trickplay`);
	} catch (error) {
		if (error instanceof ApiError && error.status === 404) return null;
		throw error;
	}
}

/** Starts loading every sheet (a few hundred KB each) so hovering is instant. */
export function preloadSheets(manifest: TrickplayManifest, token: string): void {
	for (const url of manifest.sheets) {
		const image = new Image();
		image.decoding = 'async';
		image.src = withToken(url, token);
	}
}
