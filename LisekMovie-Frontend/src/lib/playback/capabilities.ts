import type { components } from '$lib/api/schema';

export type ClientCapabilities = components['schemas']['ClientCapabilities'];

/** What the browser exposes for capability checks (injectable for tests). */
export interface MediaEnvironment {
    /** `MediaSource.isTypeSupported` (or `ManagedMediaSource` on iOS); absent without MSE. */
    isTypeSupported?: (type: string) => boolean;
    /** `HTMLVideoElement.canPlayType`. */
    canPlayType: (type: string) => string;
    /** `matchMedia(query).matches`. */
    matches: (query: string) => boolean;
    /** Output channels of the default audio device, if known. */
    audioChannels?: number;
    screen: { width: number; height: number; pixelRatio: number };
    /** hls.js works here (MSE with the codecs it needs). */
    hlsJs: boolean;
}

/** Server capability name -> MIME type with RFC 6381 codec string. */
const VIDEO: [string, string][] = [
    ['h264', 'video/mp4; codecs="avc1.640028"'],
    ['h264_10bit', 'video/mp4; codecs="avc1.6e0028"'],
    ['hevc', 'video/mp4; codecs="hvc1.1.6.L120.90"'],
    ['hevc_10bit', 'video/mp4; codecs="hvc1.2.4.L153.B0"'],
    ['av1', 'video/mp4; codecs="av01.0.08M.08"'],
    ['av1_10bit', 'video/mp4; codecs="av01.0.08M.10"'],
    ['vp9', 'video/mp4; codecs="vp09.00.10.08"'],
    ['vp9_10bit', 'video/mp4; codecs="vp09.02.10.10"']
];

const AUDIO: [string, string][] = [
    ['aac', 'audio/mp4; codecs="mp4a.40.2"'],
    ['mp3', 'audio/mp4; codecs="mp4a.6B"'],
    ['opus', 'audio/mp4; codecs="opus"'],
    ['flac', 'audio/mp4; codecs="flac"'],
    ['ac3', 'audio/mp4; codecs="ac-3"'],
    ['eac3', 'audio/mp4; codecs="ec-3"'],
    ['alac', 'audio/mp4; codecs="alac"'],
    ['dts', 'audio/mp4; codecs="dtsc"']
];

const CONTAINERS: [string, string][] = [
    ['mp4', 'video/mp4'],
    ['webm', 'video/webm'],
    ['mkv', 'video/x-matroska'],
    ['ts', 'video/mp2t']
];

/**
 * Probes what this browser decodes, in the form the server's playback
 * planner expects. Pure apart from `env`, so it can be tested.
 */
export function detectCapabilities(env: MediaEnvironment = browserEnvironment()): ClientCapabilities {
    // MSE answers for fragmented MP4 (what remux/transcode deliver); fall
    // back to the media element for browsers without MSE (older iPhones).
    const supported = (type: string) =>
        (env.isTypeSupported?.(type) ?? false) || env.canPlayType(type) === 'probably';
    const video_codecs = VIDEO.filter(([, type]) => supported(type)).map(([name]) => name);
    const audio_codecs = AUDIO.filter(([, type]) => supported(type)).map(([name]) => name);
    const containers = CONTAINERS.filter(([, type]) => env.canPlayType(type) !== '').map(([name]) => name);
    const tenBit = video_codecs.some((c) => c.endsWith('_10bit'));
    const hdr_formats: string[] = [];
    if (tenBit && env.matches('(dynamic-range: high)')) {
        hdr_formats.push('hdr10', 'hlg');
        if (env.canPlayType('video/mp4; codecs="dvh1.05.06"') !== '') hdr_formats.push('dolby_vision');
    }
    const nativeHls = env.canPlayType('application/vnd.apple.mpegurl') !== '';
    return {
        family: 'unknown',
        containers,
        video_codecs,
        audio_codecs,
        max_audio_channels: Math.max(2, Math.min(8, env.audioChannels ?? 2)),
        hdr_formats,
        hls: env.hlsJs ? 'mse' : nativeHls ? 'native' : 'none',
        interlaced: false,
        max_width: Math.round(env.screen.width * env.screen.pixelRatio),
        max_height: Math.round(env.screen.height * env.screen.pixelRatio)
    };
}

function browserEnvironment(): MediaEnvironment {
    const element = document.createElement('video');
    const source =
        (window as unknown as { ManagedMediaSource?: typeof MediaSource }).ManagedMediaSource ??
        window.MediaSource;
    let audioChannels: number | undefined;
    try {
        // Reading the device's channel count does not start audio.
        const context = new AudioContext();
        audioChannels = context.destination.maxChannelCount;
        void context.close();
    } catch {
        audioChannels = undefined;
    }
    return {
        isTypeSupported: source ? (type) => source.isTypeSupported(type) : undefined,
        canPlayType: (type) => element.canPlayType(type),
        matches: (query) => window.matchMedia?.(query).matches ?? false,
        audioChannels,
        screen: {
            width: window.screen?.width ?? 1920,
            height: window.screen?.height ?? 1080,
            pixelRatio: window.devicePixelRatio || 1
        },
        hlsJs: Boolean(source)
    };
}
