import { describe, expect, it } from 'vitest';
import { activeDevices, mergePool, normalize, simulate, switchableApi, testResult } from './pool';

const nvidia = normalize({ api: 'nvenc', device: '0', name: 'RTX 3060', enabled: true, weight: 2 });
const intel = normalize({
	api: 'qsv',
	device: '/dev/dri/renderD128',
	name: 'UHD 770',
	enabled: true,
	weight: 1,
	decode_codecs: ['h264'],
	encode_codecs: ['h264']
});

describe('transcoding pool editor', () => {
	it('spreads sessions like the server: share 2 vs 1 gives two NVIDIA, one Intel', () => {
		const { counts, cpu } = simulate([nvidia, intel], 3);
		expect(counts.get('nvenc:0')).toBe(2);
		expect(counts.get('qsv:/dev/dri/renderD128')).toBe(1);
		expect(cpu).toBe(0);
		const limited = simulate(
			[
				{ ...nvidia, max_sessions: 2 },
				{ ...intel, max_sessions: 1 }
			],
			5
		);
		expect([
			limited.counts.get('nvenc:0'),
			limited.counts.get('qsv:/dev/dri/renderD128'),
			limited.cpu
		]).toEqual([2, 1, 2]);
		expect(simulate([{ ...intel, enabled: false }], 2).cpu).toBe(2);
	});

	it('merges the saved pool with new diagnostics', () => {
		const saved = [{ ...intel, api: 'vaapi' }];
		const merged = mergePool(saved, [
			{ api: 'qsv', device: '/dev/dri/renderD128', name: 'UHD 770' },
			{ api: 'nvenc', device: '0', name: 'RTX 3060' }
		]);
		expect(merged.map((d) => `${d.api}:${d.device}`)).toEqual([
			'vaapi:/dev/dri/renderD128',
			'nvenc:0'
		]);
		expect(merged[1].enabled).toBe(false);
		expect(merged[1].encode_codecs).toEqual(['h264', 'hevc']);
	});

	it('knows switchable devices, test results and active devices', () => {
		expect(switchableApi(intel)).toBe(true);
		expect(switchableApi(nvidia)).toBe(false);
		expect(switchableApi({ ...intel, device: 'auto' })).toBe(false);
		const tests = [
			{
				encoder: 'h264_qsv',
				codec: 'h264',
				api: 'qsv',
				device: '/dev/dri/renderD128',
				ok: true,
				used_by_lisek: true
			},
			{
				encoder: 'hevc_qsv',
				codec: 'hevc',
				api: 'qsv',
				device: '/dev/dri/renderD128',
				ok: false,
				used_by_lisek: true
			}
		];
		expect(testResult(tests, intel, 'h264')).toBe(true);
		expect(testResult(tests, intel, 'hevc')).toBe(false);
		expect(testResult(tests, nvidia, 'h264')).toBeNull();
		expect(
			activeDevices([nvidia, { ...intel, decode_codecs: [], encode_codecs: [] }])
		).toHaveLength(1);
	});
});
