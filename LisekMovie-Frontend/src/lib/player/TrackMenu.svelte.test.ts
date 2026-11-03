// Faza 23 — playback settings: the quality picker offers only qualities
// below the file's own height, the subtitle background has five steps,
// and compatible playback is no longer a setting.
import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import TrackMenu from './TrackMenu.svelte';
import type { PlaybackSession } from '$lib/playback/api';

function session(sourceHeight: number | null): PlaybackSession {
	return {
		session_id: 's',
		item_id: 'i',
		title: 'Film',
		stream_url: '/s',
		position_seconds: 0,
		time_offset_seconds: 0,
		duration_seconds: 60,
		mode: 'direct_play',
		hls: false,
		reasons: [],
		delivered: {
			video_codec: 'h264',
			width: 1920,
			height: 1080,
			video_copied: true,
			tone_mapped: false,
			audio_codec: 'aac',
			audio_channels: 2,
			audio_copied: true,
			downmix: null
		},
		audio_tracks: [],
		subtitle_tracks: [],
		selected_audio_stream_index: null,
		selected_subtitle: null,
		subtitle_delivery: null,
		source_height: sourceHeight
	} as PlaybackSession;
}

function open(view: 'tracks' | 'settings', sourceHeight: number | null, quality = 'original') {
	const handlers = { onquality: vi.fn(), onbackground: vi.fn() };
	render(TrackMenu, {
		view,
		session: session(sourceHeight),
		audio: null,
		subtitle: '',
		burnIn: false,
		downmix: 'auto',
		subtitleSize: 'medium',
		subtitleBackground: 25,
		quality: quality as 'original',
		busy: false,
		stats: { bandwidthKbps: null, droppedFrames: null, bufferSeconds: 0, width: 0, height: 0 },
		onaudio: vi.fn(),
		onsubtitle: vi.fn(),
		ondownmix: vi.fn(),
		onburnin: vi.fn(),
		onsize: vi.fn(),
		onclose: vi.fn(),
		...handlers
	});
	return handlers;
}

const qualityNames = () =>
	screen
		.getAllByRole('radio')
		.filter((r) => r.closest('[aria-label="Quality"]'))
		.map((r) => r.querySelector('span')?.textContent?.trim());

describe('TrackMenu settings (Faza 23)', () => {
	it('offers only qualities below a 1080p source', async () => {
		const { onquality } = open('settings', 1080);
		expect(qualityNames()).toEqual(['Original', '720p', '480p', '360p', '144p']);
		expect(screen.queryByRole('switch', { name: /compatible/i })).toBeNull();
		await fireEvent.click(screen.getByRole('radio', { name: /^720p/ }));
		expect(onquality).toHaveBeenCalledWith('720p');
	});

	it('offers the original and 144p for a 360p source, everything for 4K', () => {
		open('settings', 360);
		expect(qualityNames()).toEqual(['Original', '144p']);
	});

	it('shows the original as chosen when the remembered quality is too big', () => {
		open('settings', 576, '720p');
		const original = screen.getByRole('radio', { name: /^Original/ });
		expect(original.getAttribute('aria-checked')).toBe('true');
	});

	it('changes the subtitle background in steps of 25%', async () => {
		const { onbackground } = open('settings', 2160);
		expect(qualityNames()).toHaveLength(7);
		const steps = screen
			.getAllByRole('radio')
			.filter((r) => r.closest('[aria-label="Subtitle background"]'));
		expect(steps.map((r) => r.getAttribute('aria-label'))).toEqual([
			'Background 0%',
			'Background 25%',
			'Background 50%',
			'Background 75%',
			'Background 100%'
		]);
		await fireEvent.click(screen.getByRole('radio', { name: 'Background 100%' }));
		expect(onbackground).toHaveBeenCalledWith(100);
	});

	it('keeps the quality out of the audio and subtitles view', () => {
		open('tracks', 1080);
		expect(screen.queryByRole('radiogroup', { name: 'Quality' })).toBeNull();
	});
});
