/**
 * The transcoding pool editor (Faza 14): the devices of
 * `hardware_pool`, merged from what the server saved and what the latest
 * diagnostics found, and a preview of how sessions spread over them —
 * the same rule as the server's balancer (`gpu::pick`): load per share,
 * `(sessions + 1) / weight`, devices at their limit skipped.
 */
import type { components } from '$lib/api/schema';

export type Report = components['schemas']['HardwareReport'];
export type EncoderTest = components['schemas']['EncoderTest'];

export interface PoolDevice {
	api: string;
	device: string;
	name: string;
	enabled: boolean;
	weight: number;
	max_sessions: number;
	decode_codecs: string[];
	encode_codecs: string[];
}

/** Every source codec a device may decode, in display order. */
export const DECODE_CODECS = [
	'h264',
	'hevc',
	'av1',
	'vp9',
	'mpeg2video',
	'vc1',
	'vp8',
	'mpeg4'
] as const;
export const ENCODE_CODECS = ['h264', 'hevc'] as const;

export function deviceId(d: Pick<PoolDevice, 'api' | 'device'>): string {
	return `${d.api}:${d.device}`;
}

/** Fills the fields the server leaves out (all have defaults there). */
export function normalize(d: Partial<PoolDevice>): PoolDevice {
	return {
		api: d.api ?? 'nvenc',
		device: d.device ?? '0',
		name: d.name ?? '',
		enabled: d.enabled ?? false,
		weight: d.weight ?? 1,
		max_sessions: d.max_sessions ?? 0,
		decode_codecs: [...(d.decode_codecs ?? [])],
		encode_codecs: [...(d.encode_codecs ?? ['h264', 'hevc'])]
	};
}

/**
 * The saved pool first (the admin's choices win), then the diagnostics'
 * suggestions for devices not in it yet. An Intel GPU saved under one API
 * is not suggested again under the other.
 */
export function mergePool(
	saved: Partial<PoolDevice>[],
	candidates: Partial<PoolDevice>[]
): PoolDevice[] {
	const list = saved.map(normalize);
	const devices = new Set(
		list.map((d) => d.device + '|' + (d.api === 'nvenc' ? 'nvidia' : 'intel'))
	);
	for (const candidate of candidates.map(normalize)) {
		const key = candidate.device + '|' + (candidate.api === 'nvenc' ? 'nvidia' : 'intel');
		if (!devices.has(key)) {
			devices.add(key);
			list.push(candidate);
		}
	}
	return list;
}

/** Whether a device's API can be switched (Intel/AMD render nodes). */
export function switchableApi(d: PoolDevice): boolean {
	return d.api !== 'nvenc' && d.device.startsWith('/dev/dri/');
}

/** The test result of `api` + `codec` on this device, if diagnostics ran. */
export function testResult(tests: EncoderTest[], d: PoolDevice, codec: string): boolean | null {
	const found = tests.filter(
		(t) => t.api === d.api && t.codec === codec && (t.device ?? '') === d.device
	);
	if (found.length === 0) return null;
	return found.some((t) => t.ok);
}

/** Devices that take part: enabled and doing something. */
export function activeDevices(list: PoolDevice[]): PoolDevice[] {
	return list.filter(
		(d) => d.enabled && (d.decode_codecs.length > 0 || d.encode_codecs.length > 0)
	);
}

/**
 * How `sessions` simultaneous transcodes spread over the active devices
 * (ties: list order, as the server's rotation starts there), and how many
 * would fall back to the CPU because every device is at its limit.
 */
export function simulate(
	list: PoolDevice[],
	sessions: number
): { counts: Map<string, number>; cpu: number } {
	const devices = activeDevices(list);
	const counts = new Map(devices.map((d) => [deviceId(d), 0]));
	let cpu = 0;
	for (let i = 0; i < sessions; i++) {
		let best: PoolDevice | null = null;
		for (const d of devices) {
			const n = counts.get(deviceId(d)) ?? 0;
			if (d.max_sessions > 0 && n >= d.max_sessions) continue;
			if (!best) {
				best = d;
				continue;
			}
			const b = counts.get(deviceId(best)) ?? 0;
			// (n + 1) / weight < (b + 1) / best.weight, without fractions.
			if ((n + 1) * Math.max(1, best.weight) < (b + 1) * Math.max(1, d.weight)) best = d;
		}
		if (best) counts.set(deviceId(best), (counts.get(deviceId(best)) ?? 0) + 1);
		else cpu++;
	}
	return { counts, cpu };
}
