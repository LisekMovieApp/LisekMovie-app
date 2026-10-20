import type { AudioTrack, PlaybackSession, SubtitleTrack } from './api';

const LAYOUTS: Record<number, string> = { 1: '1.0', 2: '2.0', 6: '5.1', 7: '6.1', 8: '7.1' };

function language(code: string | null | undefined): string {
    return code ? code.toUpperCase() : '—';
}

/** "POL · Lektor · AC3 5.1" */
export function audioLabel(track: AudioTrack): string {
    const parts = [language(track.language)];
    if (track.title) parts.push(track.title);
    const channels = track.channels ? (LAYOUTS[track.channels] ?? `${track.channels}ch`) : '';
    parts.push([track.codec?.toUpperCase(), channels].filter(Boolean).join(' '));
    return parts.filter(Boolean).join(' · ');
}

/** "POL · Forced · SRT" (`forced` is the translated word). */
export function subtitleLabel(track: SubtitleTrack, forced: string): string {
    const parts = [language(track.language)];
    if (track.title) parts.push(track.title);
    if (track.forced) parts.push(forced);
    if (track.codec) parts.push(track.codec.replace('hdmv_pgs_subtitle', 'PGS').replace('subrip', 'SRT').toUpperCase());
    return parts.join(' · ');
}

/**
 * How picking subtitle `id` ('' = off) applies to the running session:
 * switched in the player, or a new session (burn-in, or a track the
 * current delivery cannot carry).
 */
export function subtitleChange(session: PlaybackSession, id: string, burnIn: boolean): 'switch' | 'restart' {
    if (session.subtitle_delivery === 'burn_in') return 'restart';
    if (id === '') return 'switch';
    const track = session.subtitle_tracks.find((s) => s.id === id);
    if (!track || burnIn || track.kind === 'image') return 'restart';
    return (session.hls ? track.hls_rendition != null : Boolean(track.url)) ? 'switch' : 'restart';
}
