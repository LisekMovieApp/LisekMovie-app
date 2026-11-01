/**
 * Faza 20 — the viewer's picture quality. "Original" leaves the choice to
 * the server (direct play whenever the browser can); a preset asks for at
 * most that height and bitrate, which makes the server transcode (scale
 * and re-encode) anything larger, so a slow laptop or connection gets a
 * stream it can keep up with.
 */
export type QualityId = 'original' | '1080p' | '720p' | '480p' | '360p';

export interface Quality {
	id: QualityId;
	/** Largest picture height; null = the source's. */
	maxHeight: number | null;
	/** Total stream budget in kbit/s; null = no limit. */
	maxBitrateKbps: number | null;
}

export const QUALITIES: readonly Quality[] = [
	{ id: 'original', maxHeight: null, maxBitrateKbps: null },
	{ id: '1080p', maxHeight: 1080, maxBitrateKbps: 8000 },
	{ id: '720p', maxHeight: 720, maxBitrateKbps: 4000 },
	{ id: '480p', maxHeight: 480, maxBitrateKbps: 2000 },
	{ id: '360p', maxHeight: 360, maxBitrateKbps: 1000 }
];

export function isQualityId(value: unknown): value is QualityId {
	return QUALITIES.some((q) => q.id === value);
}

export function qualityOf(id: QualityId): Quality {
	return QUALITIES.find((q) => q.id === id) ?? QUALITIES[0];
}

/** The start request's limits for a quality. */
export function qualityLimits(id: QualityId): {
	max_height: number | null;
	max_bitrate_kbps: number | null;
} {
	const quality = qualityOf(id);
	return { max_height: quality.maxHeight, max_bitrate_kbps: quality.maxBitrateKbps };
}
