/**
 * Faza 20 — the viewer's picture quality. "Original" leaves the choice to
 * the server (direct play whenever the browser can); a preset asks for at
 * most that height and bitrate, which makes the server transcode (scale
 * and re-encode) anything larger, so a slow laptop or connection gets a
 * stream it can keep up with.
 */
export type QualityId = 'original' | '1440p' | '1080p' | '720p' | '480p' | '360p' | '144p';

export interface Quality {
	id: QualityId;
	/** Largest picture height; null = the source's. */
	maxHeight: number | null;
	/** Total stream budget in kbit/s; null = no limit. */
	maxBitrateKbps: number | null;
}

/** Faza 23: from 1440p down to 144p, largest first. */
export const QUALITIES: readonly Quality[] = [
	{ id: 'original', maxHeight: null, maxBitrateKbps: null },
	{ id: '1440p', maxHeight: 1440, maxBitrateKbps: 16000 },
	{ id: '1080p', maxHeight: 1080, maxBitrateKbps: 8000 },
	{ id: '720p', maxHeight: 720, maxBitrateKbps: 4000 },
	{ id: '480p', maxHeight: 480, maxBitrateKbps: 2000 },
	{ id: '360p', maxHeight: 360, maxBitrateKbps: 1000 },
	{ id: '144p', maxHeight: 144, maxBitrateKbps: 300 }
];

/**
 * Faza 23: the qualities worth offering for a file `sourceHeight` pixels
 * tall (FFprobe): the original, then only presets smaller than the
 * source — the server never upscales, so an equal or larger preset would
 * only re-encode the same picture. Unknown height: every preset.
 */
export function availableQualities(sourceHeight: number | null | undefined): Quality[] {
	return QUALITIES.filter(
		(q) =>
			q.maxHeight === null ||
			sourceHeight === null ||
			sourceHeight === undefined ||
			sourceHeight <= 0 ||
			q.maxHeight < sourceHeight
	);
}

/** The quality to show as chosen: `id` when this file offers it, else the original. */
export function effectiveQuality(
	id: QualityId,
	sourceHeight: number | null | undefined
): QualityId {
	return availableQualities(sourceHeight).some((q) => q.id === id) ? id : 'original';
}

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
