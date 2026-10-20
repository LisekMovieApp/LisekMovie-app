import { describe, expect, it } from 'vitest';
import type { PlaybackSession, SubtitleTrack } from './api';
import { audioLabel, subtitleChange, subtitleLabel } from './tracks';

const srt: SubtitleTrack = { id: 's4', kind: 'text', codec: 'subrip', language: 'pol', title: null, forced: true, is_default: false, external: false, url: null, hls_rendition: 0 };
const pgs: SubtitleTrack = { ...srt, id: 's3', kind: 'image', codec: 'hdmv_pgs_subtitle', forced: false, hls_rendition: null };
const file: SubtitleTrack = { ...srt, id: 'x0', external: true, forced: false, hls_rendition: 1, url: null };

function session(overrides: Partial<PlaybackSession>): PlaybackSession {
    return { hls: true, subtitle_tracks: [srt, pgs, file], subtitle_delivery: null, ...overrides } as PlaybackSession;
}

describe('tracks', () => {
    it('labels tracks', () => {
        expect(audioLabel({ index: 2, codec: 'ac3', language: 'pol', title: 'Lektor', channels: 6, channel_layout: '5.1', is_default: false })).toBe('POL · Lektor · AC3 5.1');
        expect(subtitleLabel(srt, 'Forced')).toBe('POL · Forced · SRT');
        expect(subtitleLabel(pgs, 'Forced')).toBe('POL · PGS');
    });

    it('switches text subtitles in place and restarts for burn-in', () => {
        expect(subtitleChange(session({}), 's4', false)).toBe('switch');
        expect(subtitleChange(session({}), '', false)).toBe('switch');
        expect(subtitleChange(session({}), 's3', false)).toBe('restart');
        expect(subtitleChange(session({}), 's4', true)).toBe('restart');
        expect(subtitleChange(session({ subtitle_delivery: 'burn_in' }), '', false)).toBe('restart');
        // Direct play: only files with a WebVTT URL switch in place.
        expect(subtitleChange(session({ hls: false }), 'x0', false)).toBe('restart');
        expect(subtitleChange(session({ hls: false, subtitle_tracks: [{ ...file, url: '/x0.vtt' }] }), 'x0', false)).toBe('switch');
    });
});
