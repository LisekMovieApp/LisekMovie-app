import { describe, expect, it } from 'vitest';
import { detectCapabilities, type MediaEnvironment } from './capabilities';

function chromeOnWindows(overrides: Partial<MediaEnvironment> = {}): MediaEnvironment {
    const mse = ['avc1.640028', 'hvc1.1.6.L120.90', 'hvc1.2.4.L153.B0', 'vp09.00.10.08', 'av01.0.08M.08', 'mp4a.40.2', 'opus', 'flac'];
    return {
        isTypeSupported: (type) => mse.some((codec) => type.includes(`"${codec}"`)),
        canPlayType: (type) => (['video/mp4', 'video/webm'].includes(type) ? 'maybe' : ''),
        matches: () => false,
        audioChannels: 2,
        screen: { width: 1920, height: 1080, pixelRatio: 1.25 },
        hlsJs: true,
        ...overrides
    };
}

describe('detectCapabilities', () => {
    it('reports MSE codecs, containers and screen size in server names', () => {
        const caps = detectCapabilities(chromeOnWindows());
        expect(caps.video_codecs).toEqual(['h264', 'hevc', 'hevc_10bit', 'av1', 'vp9']);
        expect(caps.audio_codecs).toEqual(['aac', 'opus', 'flac']);
        expect(caps.containers).toEqual(['mp4', 'webm']);
        expect(caps.hls).toBe('mse');
        expect(caps.hdr_formats).toEqual([]);
        expect([caps.max_width, caps.max_height]).toEqual([2400, 1350]);
        expect(caps.max_audio_channels).toBe(2);
    });

    it('claims HDR only with a 10-bit decoder and an HDR screen', () => {
        const hdr = detectCapabilities(chromeOnWindows({ matches: (q) => q === '(dynamic-range: high)' }));
        expect(hdr.hdr_formats).toEqual(['hdr10', 'hlg']);
        const noTenBit = detectCapabilities(
            chromeOnWindows({ matches: () => true, isTypeSupported: (t) => t.includes('avc1.640028') })
        );
        expect(noTenBit.hdr_formats).toEqual([]);
    });

    it('falls back to native HLS and the media element without MSE', () => {
        const iphone = detectCapabilities({
            canPlayType: (type) =>
                type === 'application/vnd.apple.mpegurl' || type === 'video/mp4'
                    ? 'maybe'
                    : type.includes('avc1') || type.includes('hvc1') || type.includes('ec-3')
                      ? 'probably'
                      : '',
            matches: () => false,
            audioChannels: 6,
            screen: { width: 390, height: 844, pixelRatio: 3 },
            hlsJs: false
        });
        expect(iphone.hls).toBe('native');
        expect(iphone.video_codecs).toContain('hevc_10bit');
        expect(iphone.audio_codecs).toEqual(['eac3']);
        expect(iphone.max_audio_channels).toBe(6);
    });
});
