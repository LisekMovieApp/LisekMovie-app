<!--
  Full-screen VOD player.

  Session lifecycle: `POST /api/media/{id}/playback` starts a session (the
  server picks direct play, remux or transcode for this browser). Changing
  the audio track, the stereo mix, compatible playback or burnt-in
  subtitles needs a different stream, so the player asks for a new
  session at the current position while the old one keeps playing
  ("make before break"), freezes the last frame as the poster and swaps
  the source on the same <video>: fullscreen, volume and the controls stay.
  Text subtitles switch in place (hls.js subtitle renditions or a <track>).

  Telemetry: a `ProgressReporter` per session sends
  `POST /api/telemetry/playback-progress` every 15 s while playing (30 s
  paused, as the server's answer says) and on play, pause, seek, end and
  exit, so the resume point is at most seconds old.

  Input: mouse (click = play/pause, double-click = fullscreen), touch (tap
  = controls, double-tap on a side = ±10 s), keyboard shortcuts and TV
  remotes (`keys.ts`). Seeks from repeated presses add up and run once.

  Faza 5: the seek bar previews frames from the server's trickplay sprite
  sheets, and an episode offers the next one during its end credits
  (`NextUp.svelte`, a 10 s countdown; after three episodes started with
  nobody touching anything it waits for a press). Fullscreen is taken by
  the whole document, so moving on to the next episode keeps it.
-->
<script lang="ts">
	import Hls, { type ErrorData } from 'hls.js';
	import { onMount, untrack } from 'svelte';
	import { fade } from 'svelte/transition';
	import { getLocale, t } from '$lib/i18n';
	import Icon, { type IconName } from '$lib/components/Icon.svelte';
	import {
		ApiError,
		fetchNextEpisode,
		reportProgress,
		startPlayback,
		stopPlayback,
		type DownmixPreference,
		type NextEpisode,
		type PlaybackSession,
		type StartRequest
	} from '$lib/playback/api';
	import { detectCapabilities, type ClientCapabilities } from '$lib/playback/capabilities';
	import { subtitleChange } from '$lib/playback/tracks';
	import { directionOf } from '$lib/spatial/geometry';
	import { focusables, navScope } from '$lib/spatial/spatial';
	import { activeLines, isSubtitleTrack, type CueSegment } from './cues';
	import { bufferedEnd, clampTime, endsAt, formatClock, formatTime } from './format';
	import { commandOf, seekStep, type PlayerCommand } from './keys';
	import {
		autoAdvanceAllowed,
		COUNTDOWN_SECONDS,
		nextUpDue,
		recordAutoAdvance,
		resetAutoAdvances
	} from './nextUp';
	import { loadPreferences, savePreferences, type SubtitleSize } from './preferences';
	import { SeekAccumulator } from './seek';
	import { ProgressReporter, type MediaSnapshot } from './telemetry';
	import { fetchTrickplay, preloadSheets, thumbnailAt, type TrickplayManifest } from './trickplay';
	import NextUp from './NextUp.svelte';
	import Seekbar from './Seekbar.svelte';
	import TrackMenu, { type PlaybackStats } from './TrackMenu.svelte';

	interface Props {
		id: string;
		token: string;
		/** Shown before the session answers and in the top bar. */
		heading?: string | null;
		/** Second line, e.g. "S1:E3 · Pilot". */
		subheading?: string | null;
		/** Backdrop behind the loading spinner. */
		backdrop?: string | null;
		/** Leaves the player (Back, Stop, the back arrow). */
		onexit: () => void;
		/** Plays another item (the next episode) in place of this one. */
		onnext?: (id: string) => void;
	}

	let {
		id,
		token,
		heading = null,
		subheading = null,
		backdrop = null,
		onexit,
		onnext
	}: Props = $props();

	/** Error codes with their own message; anything else is `player.failed`. */
	const KNOWN_ERRORS = new Set([
		'transcode_capacity_reached',
		'transcode_limit_exceeded',
		'transcode_unavailable',
		'media_file_unavailable',
		'media_not_found',
		'streaming_limit_too_low',
		'network_error',
		'media_error'
	]);
	const HIDE_AFTER_MS = 3000;
	const SIZE_STYLE: Record<SubtitleSize, string> = {
		small: 'clamp(15px, 2.1vw, 30px)',
		medium: 'clamp(17px, 2.7vw, 40px)',
		large: 'clamp(20px, 3.4vw, 52px)'
	};

	const prefs = loadPreferences();

	// ---- Session and media state -------------------------------------------
	let session = $state<PlaybackSession | null>(null);
	let phase = $state<'loading' | 'ready' | 'error'>('loading');
	let errorCode = $state<string | null>(null);
	/** A switch failed; the old stream keeps playing. */
	let switchError = $state<string | null>(null);
	let switching = $state(false);
	let saveFailing = $state(false);
	let video = $state<HTMLVideoElement>();
	let root = $state<HTMLDivElement>();
	let controlsEl = $state<HTMLDivElement>();
	let paused = $state(true);
	let currentTime = $state(0);
	let duration = $state(0);
	let bufferedTo = $state(0);
	let waiting = $state(false);
	let ended = $state(false);
	let volume = $state(prefs.volume);
	let muted = $state(prefs.muted);
	let fullscreen = $state(false);
	let pipSupported = $state(false);
	let finePointer = $state(true);

	// ---- Viewer choices ------------------------------------------------------
	let audio = $state<number | null>(null);
	let subtitle = $state('');
	let burnIn = $state(false);
	let downmix = $state<DownmixPreference>(prefs.downmix);
	let compatible = $state(prefs.compatible);
	let subtitleSize = $state<SubtitleSize>(prefs.subtitleSize);

	// ---- Chrome --------------------------------------------------------------
	let controls = $state(true);
	let menu = $state<'tracks' | 'settings' | null>(null);
	let pending = $state<number | null>(null);
	let scrubbing = $state(false);
	let osd = $state<{ icon: IconName; text?: string; key: number } | null>(null);
	let lines = $state<CueSegment[][]>([]);
	let resumedAt = $state<number | null>(null);
	let trick = $state<TrickplayManifest | null>(null);
	let next = $state<NextEpisode | null>(null);
	let nextDismissed = $state(false);
	/** Seconds until the next episode starts; null when not counting. */
	let countdown = $state<number | null>(null);
	let stats = $state<PlaybackStats>({
		bandwidthKbps: null,
		droppedFrames: null,
		bufferSeconds: 0,
		width: 0,
		height: 0
	});

	// ---- Plain fields (not rendered) ----------------------------------------
	let hls: Hls | null = null;
	let reporter: ProgressReporter | null = null;
	let caps: ClientCapabilities | undefined;
	let opened = 0;
	let destroyed = false;
	/** The element has data for the current session; until then reports use `startAt`. */
	let mediaReady = false;
	let startAt = 0;
	let resumePlaying = true;
	let recoveries = 0;
	let failures = 0;
	let hideTimer: ReturnType<typeof setTimeout> | undefined;
	let osdTimer: ReturnType<typeof setTimeout> | undefined;
	let resumeTimer: ReturnType<typeof setTimeout> | undefined;
	let lastTap = { time: 0, side: 0 };
	let menuOpener: HTMLElement | null = null;
	let osdKey = 0;
	let cueKey = '';
	let seekOrigin = 0;
	let sheetsWarm = false;
	/** Leaving for the next episode: keep fullscreen. */
	let continuing = false;

	const total = $derived(
		duration > 0 && Number.isFinite(duration) ? duration : (session?.duration_seconds ?? 0)
	);
	const remaining = $derived(Math.max(0, total - (pending ?? currentTime)));
	/** Wall clock for the end-time readout (Faza 8). Ticks only while the
	 * controls are on screen; paused, the end time moves on with it. */
	let now = $state(Date.now());
	const endTime = $derived.by(() => {
		if (!(total > 0)) return null;
		const end = endsAt(now, remaining);
		return end ? formatClock(end, getLocale()) : null;
	});
	const title = $derived(heading ?? session?.title ?? '');
	const volumeIcon = $derived<IconName>(
		muted || volume === 0 ? 'muted' : volume < 0.5 ? 'volume_low' : 'volume'
	);
	const showChrome = $derived(controls || paused || menu !== null || phase !== 'ready' || ended);

	const seeker = new SeekAccumulator(
		(time) => seekTo(time),
		(target) => (pending = target)
	);

	// ---- Sessions ------------------------------------------------------------

	function request(position: number | null): StartRequest {
		return {
			client: caps,
			audio_stream_index: audio,
			subtitle: subtitle || null,
			burn_subtitle: burnIn && subtitle !== '',
			force_transcode: compatible,
			downmix,
			start_position_seconds: position
		};
	}

	type Outcome = 'ok' | 'failed' | 'stale';

	/**
	 * Starts a session. `position` null = the server's resume point. A
	 * switch (`follow`) continues from wherever playback is when the new
	 * session answers; `autoplay` overrides keeping the current play state.
	 */
	async function open(
		position: number | null,
		follow = true,
		autoplay?: boolean
	): Promise<Outcome> {
		const attempt = ++opened;
		let next: PlaybackSession;
		try {
			next = await startPlayback(token, id, request(position));
		} catch (error) {
			if (attempt !== opened || destroyed) return 'stale';
			const code = error instanceof ApiError ? (error.code ?? 'internal_error') : 'network_error';
			// The server may allow one encoder per viewer: give ours back and try once more.
			if (code === 'transcode_capacity_reached' && reporter && !reporter.stopped) {
				await reporter.stop();
				return open(position, follow, autoplay);
			}
			if (session) switchError = code;
			else fail(code);
			return 'failed';
		}
		if (attempt !== opened || destroyed) {
			void stopPlayback(token, next.session_id).catch(() => {});
			return 'stale';
		}
		const first = session === null;
		const live = mediaReady ? video?.currentTime : undefined;
		const at = first ? next.position_seconds : follow ? (live ?? position ?? 0) : (position ?? 0);
		attach(next, at, !first, autoplay);
		if (first) afterFirstSession(next);
		return 'ok';
	}

	/** Trickplay sheets and the next episode; both optional extras. */
	async function loadExtras() {
		const [manifest, following] = await Promise.all([
			fetchTrickplay(token, id).catch(() => null),
			fetchNextEpisode(token, id).catch(() => null)
		]);
		if (destroyed) return;
		trick = manifest;
		next = following;
	}

	function warmSheets() {
		if (!trick || sheetsWarm) return;
		sheetsWarm = true;
		preloadSheets(trick, token);
	}

	const thumbnail = $derived.by(() => {
		const manifest = trick;
		return manifest ? (time: number) => thumbnailAt(manifest, time, token) : null;
	});

	const nextUpVisible = $derived(
		next !== null &&
			phase === 'ready' &&
			!nextDismissed &&
			nextUpDue(Math.max(0, total - currentTime), total, ended)
	);
	const counting = $derived(countdown !== null && (!paused || ended));

	/** Starts the next episode; `auto` when the countdown (not the viewer) did. */
	function goNext(auto: boolean) {
		const target = next;
		if (!target || !onnext) return;
		if (auto) recordAutoAdvance();
		else resetAutoAdvances();
		countdown = null;
		continuing = true;
		onnext(target.id);
	}

	function dismissNext() {
		nextDismissed = true;
		countdown = null;
		resetAutoAdvances();
		poke();
	}

	function afterFirstSession(next: PlaybackSession) {
		void loadExtras();
		if (next.position_seconds > 5) {
			resumedAt = next.position_seconds;
			clearTimeout(resumeTimer);
			resumeTimer = setTimeout(() => (resumedAt = null), 8000);
		}
		// The subtitle language picked last time, when it can switch in place.
		const language = prefs.subtitleLanguage;
		if (language === null) return;
		const wanted =
			language === ''
				? ''
				: next.subtitle_tracks.find(
						(s) => s.language === language && !s.forced && s.kind !== 'image'
					)?.id;
		if (
			wanted !== undefined &&
			wanted !== subtitle &&
			subtitleChange(next, wanted, false) === 'switch'
		) {
			subtitle = wanted;
			applySubtitle();
		}
	}

	/** Captures the frame on screen as the poster, so a source swap does not flash black. */
	function freezeFrame(element: HTMLVideoElement) {
		try {
			if (!element.videoWidth) return;
			const canvas = document.createElement('canvas');
			const scale = Math.min(1, 1280 / element.videoWidth);
			canvas.width = Math.round(element.videoWidth * scale);
			canvas.height = Math.round(element.videoHeight * scale);
			canvas.getContext('2d')?.drawImage(element, 0, 0, canvas.width, canvas.height);
			element.poster = canvas.toDataURL('image/jpeg', 0.8);
		} catch {
			// A cross-origin stream taints the canvas: the swap shows black for a moment.
		}
	}

	function detach() {
		mediaReady = false;
		hls?.destroy();
		hls = null;
		if (video) {
			video.removeAttribute('src');
			video.load();
		}
		lines = [];
		cueKey = '';
	}

	function attach(next: PlaybackSession, position: number, swap: boolean, autoplay?: boolean) {
		const element = video;
		if (!element) return;
		resumePlaying = autoplay ?? (swap ? !element.paused || waiting : true);
		// Final report for the old session at the switch point, then swap.
		void reporter?.stop();
		if (swap) freezeFrame(element);
		else element.removeAttribute('poster');
		detach();

		session = next;
		audio = next.selected_audio_stream_index ?? null;
		subtitle = next.selected_subtitle ?? '';
		phase = 'ready';
		switching = false;
		switchError = null;
		ended = false;
		recoveries = 0;
		startAt = clampTime(position, next.duration_seconds ?? Infinity);
		reporter = new ProgressReporter({
			sessionId: next.session_id,
			durationHint: next.duration_seconds,
			snapshot,
			send: (report, final) => reportProgress(token, report, final),
			onDelivery: (ok) => {
				failures = ok ? 0 : failures + 1;
				saveFailing = failures >= 2;
			}
		});
		reporter.start();

		if (next.hls && Hls.isSupported()) {
			const instance = new Hls({
				startPosition: startAt,
				maxBufferLength: 30,
				maxMaxBufferLength: 120,
				backBufferLength: 30,
				maxBufferSize: 120 * 1000 * 1000,
				renderTextTracksNatively: true,
				startFragPrefetch: true,
				testBandwidth: false
			});
			hls = instance;
			instance.on(Hls.Events.MANIFEST_PARSED, applySubtitle);
			instance.on(Hls.Events.SUBTITLE_TRACKS_UPDATED, applySubtitle);
			instance.on(Hls.Events.ERROR, (_event, data) => onHlsError(instance, data));
			instance.loadSource(next.stream_url);
			instance.attachMedia(element);
		} else {
			// Direct play, or HLS the browser plays itself (Safari, TVs).
			element.src = next.stream_url;
		}
	}

	/** Errors no retry can fix: the stream's codecs are not playable here. */
	const UNPLAYABLE = new Set<string>([
		Hls.ErrorDetails.MANIFEST_INCOMPATIBLE_CODECS_ERROR,
		Hls.ErrorDetails.BUFFER_INCOMPATIBLE_CODECS_ERROR,
		Hls.ErrorDetails.BUFFER_ADD_CODEC_ERROR
	]);

	function onHlsError(instance: Hls, data: ErrorData) {
		if (instance !== hls) return;
		if (UNPLAYABLE.has(data.details)) return fail('media_error');
		if (!data.fatal) return;
		// One retry per kind, as hls.js recommends; then give up.
		if (data.type === Hls.ErrorTypes.NETWORK_ERROR && recoveries < 3) {
			recoveries += 1;
			instance.startLoad(video?.currentTime ?? -1);
		} else if (data.type === Hls.ErrorTypes.MEDIA_ERROR && recoveries < 3) {
			recoveries += 1;
			if (recoveries === 2) instance.swapAudioCodec();
			instance.recoverMediaError();
		} else {
			fail(data.type === Hls.ErrorTypes.NETWORK_ERROR ? 'network_error' : 'media_error');
		}
	}

	function fail(code: string) {
		phase = 'error';
		errorCode = code;
		switching = false;
		void reporter?.stop();
		hls?.stopLoad();
	}

	/** `compatibleMode`: retry as the server's safest stream (H.264/AAC transcode). */
	function retry(compatibleMode = false) {
		if (compatibleMode) {
			compatible = true;
			prefs.compatible = true;
			savePreferences(prefs);
		}
		phase = 'loading';
		errorCode = null;
		const position = video && mediaReady ? video.currentTime : startAt;
		session = null;
		detach();
		void open(position || null, false, true);
	}

	/** Applies a choice that needs a new stream; reverts it when the switch fails. */
	async function reopen(change: () => void) {
		if (!session) return;
		const before = { audio, subtitle, burnIn, downmix, compatible };
		change();
		switching = true;
		switchError = null;
		const outcome = await open(mediaReady && video ? video.currentTime : startAt);
		// A newer choice superseded this one ('stale'): keep that one's values.
		if (outcome === 'failed' && !destroyed) {
			({ audio, subtitle, burnIn, downmix, compatible } = before);
			switching = false;
		}
	}

	// ---- Tracks --------------------------------------------------------------

	function applySubtitle() {
		const current = session;
		if (!current) return;
		const track = current.subtitle_tracks.find((s) => s.id === subtitle);
		const burnt = current.subtitle_delivery === 'burn_in';
		const rendition = burnt ? -1 : (track?.hls_rendition ?? -1);
		if (hls) {
			hls.subtitleDisplay = false;
			hls.subtitleTrack = rendition;
		} else if (video) {
			const tracks = Array.from(video.textTracks).filter(isSubtitleTrack);
			// Native HLS lists one text track per rendition, in order; direct
			// play has at most the one <track> below.
			tracks.forEach((tt, i) => {
				const on = current.hls ? i === rendition : Boolean(track?.url) && !burnt;
				tt.mode = on ? 'hidden' : 'disabled';
			});
		}
		refreshCues();
	}

	function chooseSubtitle(next: string) {
		if (!session || next === subtitle) return;
		const language =
			next === '' ? '' : (session.subtitle_tracks.find((s) => s.id === next)?.language ?? null);
		prefs.subtitleLanguage = language;
		savePreferences(prefs);
		if (subtitleChange(session, next, burnIn) === 'switch') {
			subtitle = next;
			applySubtitle();
		} else {
			void reopen(() => (subtitle = next));
		}
	}

	function chooseAudio(index: number) {
		if (index !== audio) void reopen(() => (audio = index));
	}

	function chooseDownmix(mode: DownmixPreference) {
		prefs.downmix = mode;
		savePreferences(prefs);
		if (mode !== downmix) void reopen(() => (downmix = mode));
	}

	function chooseCompatible(value: boolean) {
		prefs.compatible = value;
		savePreferences(prefs);
		void reopen(() => (compatible = value));
	}

	function chooseBurnIn(value: boolean) {
		if (subtitle === '') burnIn = value;
		else void reopen(() => (burnIn = value));
	}

	function chooseSize(size: SubtitleSize) {
		subtitleSize = size;
		prefs.subtitleSize = size;
		savePreferences(prefs);
	}

	/** `c`: subtitles off, or back on in the last language. */
	function toggleSubtitles() {
		if (!session) return;
		if (subtitle !== '') return chooseSubtitle('');
		const language = prefs.subtitleLanguage;
		const candidates = session.subtitle_tracks.filter((s) => s.kind !== 'image' && !s.forced);
		const next = candidates.find((s) => language && s.language === language) ?? candidates[0];
		if (next) chooseSubtitle(next.id);
	}

	function refreshCues() {
		const next = video ? activeLines(video.textTracks) : [];
		const key = JSON.stringify(next);
		if (key !== cueKey) {
			cueKey = key;
			lines = next;
		}
	}

	function watchTextTracks(element: HTMLVideoElement) {
		const bound = new WeakSet<TextTrack>();
		// iPhone fullscreen hands the picture to the system player, which
		// only shows cues the browser draws.
		let native = false;
		const bind = () => {
			for (const track of Array.from(element.textTracks)) {
				// The player draws cues itself; the browser must not draw them too.
				if (!native && isSubtitleTrack(track) && track.mode === 'showing') track.mode = 'hidden';
				if (!bound.has(track)) {
					bound.add(track);
					track.addEventListener('cuechange', refreshCues);
				}
			}
			refreshCues();
		};
		const onAdd = () => {
			bind();
			if (!hls) applySubtitle();
		};
		const setNative = (on: boolean) => () => {
			native = on;
			for (const track of Array.from(element.textTracks)) {
				if (isSubtitleTrack(track) && track.mode !== 'disabled')
					track.mode = on ? 'showing' : 'hidden';
			}
		};
		const begin = setNative(true);
		const end = setNative(false);
		element.textTracks.addEventListener('addtrack', onAdd);
		element.textTracks.addEventListener('change', bind);
		element.addEventListener('webkitbeginfullscreen', begin);
		element.addEventListener('webkitendfullscreen', end);
		return () => {
			element.textTracks.removeEventListener('addtrack', onAdd);
			element.textTracks.removeEventListener('change', bind);
			element.removeEventListener('webkitbeginfullscreen', begin);
			element.removeEventListener('webkitendfullscreen', end);
		};
	}

	// ---- Telemetry snapshot ----------------------------------------------------

	function snapshot(): MediaSnapshot {
		const element = video;
		if (!element || !mediaReady) {
			return { currentTime: startAt, duration: session?.duration_seconds ?? 0, paused: true };
		}
		const quality = element.getVideoPlaybackQuality?.();
		return {
			currentTime: element.currentTime,
			duration: element.duration,
			paused: element.paused,
			bufferedSeconds: Math.max(
				0,
				bufferedEnd(element.buffered, element.currentTime) - element.currentTime
			),
			droppedFrames: quality?.droppedVideoFrames,
			bandwidthKbps:
				hls && Number.isFinite(hls.bandwidthEstimate) ? hls.bandwidthEstimate / 1000 : undefined
		};
	}

	function updateStats() {
		const now = snapshot();
		stats = {
			bandwidthKbps: now.bandwidthKbps ?? null,
			droppedFrames: now.droppedFrames ?? null,
			bufferSeconds: now.bufferedSeconds ?? 0,
			width: video?.videoWidth ?? 0,
			height: video?.videoHeight ?? 0
		};
	}

	// ---- Playback controls ----------------------------------------------------

	function play() {
		if (!video) return;
		if (ended) {
			// The ended session is closed on the server: a new one, from the top.
			ended = false;
			switching = true;
			void open(0, false, true);
			return;
		}
		video.play().catch(() => (paused = true));
	}

	function togglePlay() {
		if (!video || phase !== 'ready') return;
		if (video.paused || ended) {
			play();
			flash('play');
		} else {
			video.pause();
			flash('pause');
		}
	}

	function seekTo(time: number) {
		if (!video) return;
		ended = false;
		if (!mediaReady) {
			startAt = time;
			if (hls) hls.startLoad(time);
			return;
		}
		video.currentTime = time;
	}

	function seekBy(step: number, repeat = false) {
		if (!video || phase !== 'ready' || !(total > 0)) return;
		warmSheets();
		const from = video.currentTime;
		// The jump is shown from where the first press of this burst was.
		if (seeker.target === null) seekOrigin = from;
		const target = seeker.add(from, step, total, repeat);
		const delta = target - seekOrigin;
		flash(
			step < 0 ? 'rewind' : 'forward',
			`${delta < 0 ? '−' : '+'}${formatTime(Math.abs(delta))}`,
			1200
		);
		poke();
	}

	function scrub(time: number, active: boolean) {
		scrubbing = active;
		seeker.set(time, total, !active);
		poke();
	}

	function setVolume(value: number) {
		if (!video) return;
		video.volume = Math.min(1, Math.max(0, Math.round(value * 100) / 100));
		video.muted = video.volume === 0;
	}

	function changeVolume(delta: number) {
		if (!video) return;
		setVolume((video.muted ? 0 : video.volume) + delta);
		flash(video.volume === 0 ? 'muted' : 'volume', `${Math.round(video.volume * 100)}%`);
	}

	function toggleMute() {
		if (!video) return;
		video.muted = !video.muted;
		if (!video.muted && video.volume === 0) video.volume = 0.5;
		flash(video.muted ? 'muted' : 'volume');
	}

	async function toggleFullscreen() {
		if (document.fullscreenElement) {
			await document.exitFullscreen().catch(() => {});
			return;
		}
		// The whole document, not the player: the next episode mounts a new
		// player and must stay fullscreen.
		const page = document.documentElement;
		if (page.requestFullscreen) {
			await page.requestFullscreen({ navigationUI: 'hide' }).catch(() => {});
			// Phones: turn the picture sideways with the screen.
			const orientation = screen.orientation as ScreenOrientation & {
				lock?: (o: string) => Promise<void>;
			};
			await orientation?.lock?.('landscape').catch(() => {});
		} else {
			// iPhone Safari only lets the <video> itself go fullscreen.
			(
				video as HTMLVideoElement & { webkitEnterFullscreen?: () => void }
			)?.webkitEnterFullscreen?.();
		}
	}

	async function togglePip() {
		if (!video) return;
		if (document.pictureInPictureElement) await document.exitPictureInPicture().catch(() => {});
		else await video.requestPictureInPicture().catch(() => {});
	}

	function flash(icon: IconName, text?: string, ms = 700) {
		osd = { icon, text, key: ++osdKey };
		clearTimeout(osdTimer);
		osdTimer = setTimeout(() => (osd = null), ms);
	}

	// ---- Controls visibility --------------------------------------------------

	/** Shows the controls and restarts the hide timer. */
	function poke() {
		controls = true;
		clearTimeout(hideTimer);
		hideTimer = setTimeout(hideControls, HIDE_AFTER_MS);
	}

	function hideControls() {
		if (paused || menu || scrubbing || pending !== null || phase !== 'ready' || ended) return;
		controls = false;
		// Focus must not stay on a control nobody can see.
		if (controlsEl?.contains(document.activeElement)) root?.focus({ preventScroll: true });
	}

	/** Shows the controls and puts focus on them (remote OK / Up / Down). */
	function revealControls() {
		poke();
		requestAnimationFrame(() => {
			if (!controlsEl || controlsEl.contains(document.activeElement)) return;
			const target =
				controlsEl.querySelector<HTMLElement>('[data-nav-default]') ?? focusables(controlsEl)[0];
			target?.focus({ preventScroll: true });
		});
	}

	function openMenu(view: 'tracks' | 'settings', event?: Event) {
		menuOpener = (event?.currentTarget as HTMLElement | null) ?? null;
		menu = menu === view ? null : view;
		if (menu === 'settings') updateStats();
		poke();
	}

	function closeMenu() {
		menu = null;
		menuOpener?.focus({ preventScroll: true });
		poke();
	}

	function back(escape: boolean) {
		if (menu) return closeMenu();
		if (escape) {
			if (controls && !paused) hideControls();
			return;
		}
		// Remote Back: first hide the controls, then leave.
		if (controls && !paused && !finePointer) return hideControls();
		onexit();
	}

	function run(command: PlayerCommand, repeat: boolean) {
		switch (command) {
			case 'toggle':
				return togglePlay();
			case 'play':
				if (video?.paused) togglePlay();
				return;
			case 'pause':
				if (video && !video.paused) togglePlay();
				return;
			case 'stop':
				return onexit();
			case 'rewind':
			case 'forward':
			case 'rewind_long':
			case 'forward_long':
				return seekBy(seekStep(command), repeat);
			case 'volume_up':
				return changeVolume(0.1);
			case 'volume_down':
				return changeVolume(-0.1);
			case 'mute':
				return toggleMute();
			case 'fullscreen':
				return void toggleFullscreen();
			case 'subtitles':
				return toggleSubtitles();
			case 'back':
				return;
		}
	}

	// Registered in the capture phase: runs before spatial navigation, which
	// skips events this handler prevented.
	function onKeyDown(event: KeyboardEvent) {
		if (event.defaultPrevented) return;
		resetAutoAdvances();
		const target = event.target instanceof HTMLElement ? event.target : null;
		// The next-episode card: its buttons take arrows and OK like controls.
		const inCard = Boolean(target?.closest('[data-player-ui]'));
		const inControls = Boolean(target && controlsEl?.contains(target)) || inCard;
		const command = commandOf(event);
		if (command === 'back') {
			event.preventDefault();
			return back(event.key === 'Escape');
		}
		if (phase !== 'ready') return;
		if (menu) {
			// Arrows move inside the panel (spatial navigation).
			poke();
			return;
		}
		if (command) {
			// Space/Enter on a focused button presses that button.
			if (command === 'toggle' && event.key === ' ' && target instanceof HTMLButtonElement)
				return poke();
			event.preventDefault();
			run(command, event.repeat);
			return;
		}
		const direction = directionOf(event);
		if (direction) {
			if ((controls || inCard) && inControls) return poke();
			event.preventDefault();
			if (direction === 'left' || direction === 'right')
				return seekBy(direction === 'left' ? -10 : 10, event.repeat);
			if (finePointer) return changeVolume(direction === 'up' ? 0.1 : -0.1);
			return revealControls();
		}
		if (event.key === 'Enter' && !inControls) {
			event.preventDefault();
			togglePlay();
			if (!finePointer) revealControls();
			return;
		}
		poke();
	}

	function onSurfacePointerUp(event: PointerEvent) {
		resetAutoAdvances();
		if (phase !== 'ready' || event.button !== 0) return;
		if (menu) return closeMenu();
		if (event.pointerType === 'mouse') return togglePlay();
		// Touch: tap shows/hides the controls; a double tap on a side jumps 10 s.
		const width = (event.currentTarget as HTMLElement).clientWidth;
		const side = event.offsetX < width / 3 ? -1 : event.offsetX > (width * 2) / 3 ? 1 : 0;
		const now = performance.now();
		if (side !== 0 && now - lastTap.time < 300 && lastTap.side === side) {
			seekBy(side * 10);
			lastTap = { time: 0, side: 0 };
			return;
		}
		lastTap = { time: now, side };
		if (controls && !paused) hideControls();
		else poke();
	}

	// ---- Media element events --------------------------------------------------

	function onLoadedMetadata() {
		if (!video) return;
		// hls.js starts at `startPosition` itself; direct play seeks here.
		if (!hls && startAt > 0 && Math.abs(video.currentTime - startAt) > 1) {
			video.currentTime = Math.min(startAt, Math.max(0, video.duration - 1));
		}
		applySubtitle();
	}

	function onCanPlay() {
		if (!video || mediaReady) return;
		mediaReady = true;
		if (resumePlaying) play();
	}

	function onTimeUpdate() {
		if (!video) return;
		currentTime = video.currentTime;
		bufferedTo = bufferedEnd(video.buffered, video.currentTime);
		refreshCues();
	}

	function onVolumeChange() {
		if (!video) return;
		volume = video.volume;
		muted = video.muted;
		prefs.volume = volume;
		prefs.muted = muted;
		savePreferences(prefs);
	}

	function onEnded() {
		ended = true;
		paused = true;
		controls = true;
		reporter?.onEnded();
	}

	function onMediaError() {
		// hls.js reports its own errors; this is direct play or native HLS.
		if (!hls && session) fail('media_error');
	}

	// ---- Effects -----------------------------------------------------------------

	$effect(() => {
		// Paused, menus and seeks keep the controls up; otherwise they hide.
		if (paused || menu || pending !== null) {
			controls = true;
			clearTimeout(hideTimer);
		} else {
			poke();
		}
	});

	$effect(() => {
		// The card appears: count down unless the binge guard says wait.
		if (!nextUpVisible) {
			countdown = null;
			return;
		}
		if (untrack(() => countdown) === null && autoAdvanceAllowed()) countdown = COUNTDOWN_SECONDS;
	});

	$effect(() => {
		if (!counting) return;
		const step = 0.25;
		const timer = setInterval(() => {
			if (countdown === null) return;
			countdown = Math.max(0, countdown - step);
			if (countdown === 0) goNext(true);
		}, step * 1000);
		return () => clearInterval(timer);
	});

	$effect(() => {
		if (!showChrome) return;
		now = Date.now();
		const timer = setInterval(() => (now = Date.now()), 1000);
		return () => clearInterval(timer);
	});

	$effect(() => {
		if (menu !== 'settings') return;
		const timer = setInterval(updateStats, 1000);
		return () => clearInterval(timer);
	});

	$effect(() => {
		// Lock screen, headphones and hardware media keys.
		const media = typeof navigator !== 'undefined' ? navigator.mediaSession : undefined;
		if (!media || !session) return;
		try {
			media.metadata = new MediaMetadata({
				title: title,
				artist: subheading ?? '',
				artwork: backdrop ? [{ src: backdrop }] : []
			});
		} catch {
			// MediaMetadata missing (older TVs).
		}
	});

	onMount(() => {
		const element = video;
		if (!element) return;
		destroyed = false;
		finePointer = window.matchMedia?.('(pointer: fine)').matches ?? true;
		pipSupported = Boolean(document.pictureInPictureEnabled);
		element.volume = volume;
		element.muted = muted;
		caps = detectCapabilities();

		const unwatch = watchTextTracks(element);
		const onFullscreen = () => (fullscreen = Boolean(document.fullscreenElement));
		const onPageHide = () => void reporter?.stop();
		const onVisibility = () => {
			// A phone may never come back from the background: save now.
			if (document.visibilityState === 'hidden') void reporter?.report('heartbeat', true);
		};
		window.addEventListener('keydown', onKeyDown, { capture: true });
		document.addEventListener('fullscreenchange', onFullscreen);
		window.addEventListener('pagehide', onPageHide);
		document.addEventListener('visibilitychange', onVisibility);

		const media = navigator.mediaSession;
		const handlers: [MediaSessionAction, MediaSessionActionHandler][] = [
			['play', () => play()],
			['pause', () => video?.pause()],
			['seekbackward', () => seekBy(-10)],
			['seekforward', () => seekBy(10)],
			['seekto', (details) => details.seekTime !== undefined && seekTo(details.seekTime)],
			['nexttrack', () => goNext(false)],
			['stop', () => onexit()]
		];
		for (const [action, handler] of handlers) {
			try {
				media?.setActionHandler(action, handler);
			} catch {
				// Action not supported here.
			}
		}

		if (!token) fail('invalid_token');
		else void open(null);

		return () => {
			destroyed = true;
			opened++;
			seeker.cancel();
			void reporter?.stop();
			reporter = null;
			detach();
			unwatch();
			clearTimeout(hideTimer);
			clearTimeout(osdTimer);
			clearTimeout(resumeTimer);
			window.removeEventListener('keydown', onKeyDown, { capture: true });
			document.removeEventListener('fullscreenchange', onFullscreen);
			window.removeEventListener('pagehide', onPageHide);
			document.removeEventListener('visibilitychange', onVisibility);
			for (const [action] of handlers) {
				try {
					media?.setActionHandler(action, null);
				} catch {
					// Ignore.
				}
			}
			if (document.fullscreenElement && !continuing) {
				void document.exitFullscreen().catch(() => {});
			}
		};
	});

	const directSubtitle = $derived(
		session && !session.hls && session.subtitle_delivery !== 'burn_in'
			? session.subtitle_tracks.find((s) => s.id === subtitle && s.url)
			: undefined
	);
	const errorKey = $derived(
		errorCode && KNOWN_ERRORS.has(errorCode) ? `player.errors.${errorCode}` : 'player.failed'
	);
	const button =
		'grid size-11 shrink-0 place-items-center rounded-full text-white/90 transition hover:scale-110 hover:text-white focus-visible:scale-110 focus-visible:bg-white/15 md:size-12';
</script>

<div
	bind:this={root}
	class="fixed inset-0 z-[60] overflow-hidden bg-black text-white outline-none select-none focus-visible:outline-none {showChrome
		? ''
		: 'cursor-none'}"
	role="region"
	aria-label={t('player.title')}
	tabindex="-1"
	use:navScope
	onpointermove={(event) => {
		if (event.pointerType === 'mouse') poke();
	}}
>
	<!-- Subtitles are drawn by the player (below), not by the browser. -->
	<video
		bind:this={video}
		class="absolute inset-0 size-full bg-black object-contain"
		playsinline
		preload="auto"
		crossorigin="anonymous"
		onloadedmetadata={onLoadedMetadata}
		oncanplay={onCanPlay}
		ontimeupdate={onTimeUpdate}
		ondurationchange={() => (duration = video?.duration ?? 0)}
		onprogress={() => video && (bufferedTo = bufferedEnd(video.buffered, video.currentTime))}
		onplay={() => {
			paused = false;
			if (mediaReady) reporter?.onPlay();
		}}
		onpause={() => {
			paused = true;
			if (mediaReady) reporter?.onPause();
		}}
		onseeking={() => {
			waiting = true;
			if (mediaReady) reporter?.onSeeking();
		}}
		onseeked={() => {
			waiting = false;
			if (mediaReady) reporter?.onSeeked();
		}}
		onwaiting={() => {
			waiting = true;
			if (mediaReady) reporter?.onWaiting();
		}}
		onplaying={() => {
			waiting = false;
			if (mediaReady) reporter?.onPlaying();
		}}
		onended={onEnded}
		onvolumechange={onVolumeChange}
		onerror={onMediaError}
	>
		{#if directSubtitle?.url}
			{#key directSubtitle.id}
				<track
					kind="subtitles"
					src={directSubtitle.url}
					srclang={directSubtitle.language ?? undefined}
				/>
			{/key}
		{/if}
	</video>

	<!-- Click / tap surface over the picture (keyboard users have the controls). -->
	<div
		class="absolute inset-0"
		role="presentation"
		onpointerup={onSurfacePointerUp}
		ondblclick={() => finePointer && toggleFullscreen()}
	></div>

	<!-- Subtitles, lifted above the controls while they are shown. -->
	{#if lines.length}
		<div
			class="pointer-events-none absolute inset-x-0 flex flex-col items-center gap-[0.15em] px-[6%] text-center leading-tight font-semibold transition-[bottom] duration-300 [text-shadow:0_0_4px_rgb(0_0_0/0.95),0_2px_6px_rgb(0_0_0/0.9)]"
			style:bottom={showChrome ? 'max(9rem, 16%)' : '7%'}
			style:font-size={SIZE_STYLE[subtitleSize]}
			aria-live="off"
		>
			{#each lines as line, i (i)}
				<p class="rounded-sm bg-black/25 px-[0.35em] whitespace-pre-wrap">
					{#each line as segment, j (j)}<span
							class:italic={segment.italic}
							class:font-extrabold={segment.bold}
							class:underline={segment.underline}>{segment.text}</span
						>{/each}
				</p>
			{/each}
		</div>
	{/if}

	<!-- Loading / buffering / switching -->
	{#if phase === 'loading'}
		<div class="absolute inset-0 grid place-items-center" transition:fade={{ duration: 200 }}>
			{#if backdrop}
				<img
					src={backdrop}
					alt=""
					class="absolute inset-0 size-full scale-105 object-cover opacity-30 blur-sm"
				/>
			{/if}
			<div class="relative flex flex-col items-center gap-6 px-6 text-center">
				<span
					class="size-14 animate-spin rounded-full border-4 border-white/15 border-t-gold"
					role="status"
					aria-label={t('common.loading')}
				></span>
				{#if title}<p class="text-2xl font-bold md:text-4xl">{title}</p>{/if}
				{#if subheading}<p class="text-neutral-300">{subheading}</p>{/if}
			</div>
		</div>
	{:else if phase === 'ready' && waiting && !switching}
		<div class="pointer-events-none absolute inset-0 grid place-items-center">
			<span
				class="buffering size-14 animate-spin rounded-full border-4 border-white/15 border-t-gold"
				role="status"
				aria-label={t('player.buffering')}
			></span>
		</div>
	{/if}

	{#if osd}
		{#key osd.key}
			<div
				class="pointer-events-none absolute inset-0 grid place-items-center"
				out:fade={{ duration: 250 }}
			>
				<div
					class="osd flex flex-col items-center gap-2 rounded-full bg-black/55 p-5 backdrop-blur-sm"
				>
					<Icon name={osd.icon} class="size-10 md:size-14" />
					{#if osd.text}<span class="text-lg font-bold tabular-nums md:text-2xl">{osd.text}</span
						>{/if}
				</div>
			</div>
		{/key}
	{/if}

	{#if phase === 'error'}
		<div
			class="absolute inset-0 grid place-items-center bg-black/80 p-6"
			role="alert"
			use:navScope={{ autofocus: true }}
		>
			<div class="max-w-lg rounded-2xl bg-surface p-8 text-center ring-1 ring-white/10">
				<p class="mb-2 text-xl font-semibold">{title}</p>
				<p class="mb-6 text-neutral-300">{t(errorKey)}</p>
				<div class="flex justify-center gap-3">
					<button
						type="button"
						class="rounded-md bg-gold px-6 py-3 font-semibold text-black hover:bg-gold-dim"
						data-nav-default
						onclick={() => retry()}>{t('player.retry')}</button
					>
					{#if errorCode === 'media_error' && !compatible}
						<button
							type="button"
							class="rounded-md bg-white/10 px-6 py-3 font-semibold hover:bg-white/20"
							onclick={() => retry(true)}>{t('player.try_compatible')}</button
						>
					{/if}
					<button
						type="button"
						class="rounded-md bg-white/10 px-6 py-3 font-semibold hover:bg-white/20"
						onclick={onexit}>{t('common.back')}</button
					>
				</div>
			</div>
		</div>
	{/if}

	<!-- Controls -->
	<div
		bind:this={controlsEl}
		class="pointer-events-none absolute inset-0 flex flex-col justify-between transition-opacity duration-300 {showChrome &&
		phase !== 'error'
			? 'opacity-100'
			: 'opacity-0'}"
		inert={!showChrome || phase === 'error'}
	>
		<div
			class="pointer-events-auto flex items-start gap-3 bg-gradient-to-b from-black/85 via-black/45 to-transparent px-4 pt-4 pb-20 [text-shadow:0_1px_3px_rgb(0_0_0/0.8)] md:px-8 md:pt-6"
		>
			<button type="button" class={button} aria-label={t('common.back')} onclick={onexit}>
				<Icon name="back" class="size-7" />
			</button>
			<div class="min-w-0 pt-1.5">
				<p class="truncate text-lg font-bold md:text-2xl">{title}</p>
				{#if subheading}<p class="truncate text-sm text-neutral-200 md:text-[1rem]">
						{subheading}
					</p>{/if}
			</div>
			<div class="ml-auto flex shrink-0 flex-col items-end gap-2 pt-2 text-sm">
				{#if switching}
					<span
						class="flex items-center gap-2 rounded-full bg-black/60 px-3 py-1.5 text-gold"
						role="status"
					>
						<span class="size-3.5 animate-spin rounded-full border-2 border-gold/30 border-t-gold"
						></span>
						{t('player.switching')}
					</span>
				{/if}
				{#if switchError}
					<span class="rounded-full bg-black/60 px-3 py-1.5 text-gold" role="alert">
						{t(
							KNOWN_ERRORS.has(switchError)
								? `player.errors.${switchError}`
								: 'player.switch_failed'
						)}
					</span>
				{/if}
				{#if saveFailing}
					<span class="rounded-full bg-black/60 px-3 py-1.5 text-neutral-300" role="status"
						>{t('player.progress_not_saved')}</span
					>
				{/if}
			</div>
		</div>

		{#if ended}
			<div class="pointer-events-auto grid place-items-center">
				<button
					type="button"
					class="flex items-center gap-3 rounded-md bg-white px-8 py-3.5 text-lg font-bold text-black hover:bg-white/85"
					data-nav-default
					onclick={play}
				>
					<Icon name="play" class="size-6" />{t('player.replay')}
				</button>
			</div>
		{/if}

		<div
			class="pointer-events-auto bg-gradient-to-t from-black via-black/65 to-transparent px-4 pt-28 pb-4 [text-shadow:0_1px_3px_rgb(0_0_0/0.8)] md:px-8 md:pb-6"
			class:invisible={phase !== 'ready'}
		>
			{#if resumedAt !== null && phase === 'ready'}
				<div class="mb-4 flex items-center gap-3" transition:fade={{ duration: 200 }}>
					<span class="text-sm text-neutral-300"
						>{t('player.resumed', { time: formatTime(resumedAt) })}</span
					>
					<button
						type="button"
						class="rounded-full bg-white/15 px-4 py-1.5 text-sm font-semibold hover:bg-white/25"
						onclick={() => {
							resumedAt = null;
							seekTo(0);
						}}>{t('player.start_over')}</button
					>
				</div>
			{/if}
			<div class="flex items-center gap-4">
				<Seekbar
					{currentTime}
					duration={total}
					{bufferedTo}
					{pending}
					onscrub={scrub}
					oncommit={() => seeker.flush()}
					onstep={(seconds, repeat) => seekBy(seconds, repeat)}
					ontoggle={togglePlay}
					navDefault={!finePointer}
					{thumbnail}
					onwarm={warmSheets}
				/>
				<span class="w-20 shrink-0 text-right text-sm text-white tabular-nums md:text-[1rem]"
					>−{formatTime(remaining)}</span
				>
			</div>
			<div class="mt-1 flex items-center gap-1 md:gap-2">
				<button
					type="button"
					class={button}
					aria-label={paused ? t('player.play') : t('player.pause')}
					data-nav-default={finePointer ? '' : undefined}
					onclick={togglePlay}
				>
					<Icon name={paused ? 'play' : 'pause'} class="size-7 md:size-8" />
				</button>
				<button
					type="button"
					class={button}
					aria-label={t('player.rewind')}
					onclick={() => seekBy(-10)}
				>
					<Icon name="rewind" class="size-7" />
				</button>
				<button
					type="button"
					class={button}
					aria-label={t('player.forward')}
					onclick={() => seekBy(10)}
				>
					<Icon name="forward" class="size-7" />
				</button>
				<div class="group/volume flex items-center">
					<button
						type="button"
						class={button}
						aria-label={muted ? t('player.unmute') : t('player.mute')}
						onclick={toggleMute}
					>
						<Icon name={volumeIcon} class="size-7" />
					</button>
					<input
						type="range"
						min="0"
						max="1"
						step="0.05"
						value={muted ? 0 : volume}
						aria-label={t('player.volume')}
						class="volume hidden w-24 pointer-fine:block"
						oninput={(event) => setVolume(Number(event.currentTarget.value))}
					/>
				</div>
				<span class="ml-2 hidden text-sm text-neutral-100 tabular-nums sm:inline md:text-[1rem]">
					{formatTime(pending ?? currentTime)} / {formatTime(total)}
				</span>
				{#if endTime}
					<span
						class="ml-1 hidden text-sm text-neutral-300 tabular-nums md:inline md:text-[1rem]"
						data-testid="player-ends-at"
						>{t('player.ends_at', { time: endTime })}</span
					>
				{/if}
				<div class="ml-auto flex items-center gap-1 md:gap-2">
					{#if next && onnext}
						<button
							type="button"
							class={button}
							aria-label={t('player.next_episode')}
							title={t('player.next_episode')}
							onclick={() => goNext(false)}
						>
							<Icon name="next" class="size-7" />
						</button>
					{/if}
					{#if session && (session.audio_tracks.length > 1 || session.subtitle_tracks.length > 0)}
						<button
							type="button"
							class="{button} {menu === 'tracks' ? 'text-gold' : ''}"
							aria-label={t('player.tracks')}
							aria-expanded={menu === 'tracks'}
							onclick={(event) => openMenu('tracks', event)}
						>
							<Icon name="subtitles" class="size-7" />
						</button>
					{/if}
					<button
						type="button"
						class="{button} {menu === 'settings' ? 'text-gold' : ''}"
						aria-label={t('player.settings')}
						aria-expanded={menu === 'settings'}
						onclick={(event) => openMenu('settings', event)}
					>
						<Icon name="tune" class="size-7" />
					</button>
					{#if pipSupported}
						<button
							type="button"
							class="{button} hidden pointer-fine:grid"
							aria-label={t('player.pip')}
							onclick={togglePip}
						>
							<Icon name="pip" class="size-7" />
						</button>
					{/if}
					<button
						type="button"
						class={button}
						aria-label={fullscreen ? t('player.exit_fullscreen') : t('player.fullscreen')}
						onclick={toggleFullscreen}
					>
						<Icon name={fullscreen ? 'fullscreen_exit' : 'fullscreen'} class="size-7" />
					</button>
				</div>
			</div>
		</div>
	</div>

	{#if nextUpVisible && next && onnext && !menu}
		<NextUp
			{next}
			{token}
			{countdown}
			autofocus={!finePointer}
			onplay={() => goNext(false)}
			ondismiss={dismissNext}
		/>
	{/if}

	{#if menu && session}
		<TrackMenu
			view={menu}
			{session}
			{audio}
			{subtitle}
			{burnIn}
			{downmix}
			{compatible}
			{subtitleSize}
			busy={switching}
			{stats}
			onaudio={chooseAudio}
			onsubtitle={chooseSubtitle}
			ondownmix={chooseDownmix}
			oncompatible={chooseCompatible}
			onburnin={chooseBurnIn}
			onsize={chooseSize}
			onclose={closeMenu}
		/>
	{/if}
</div>

<style>
	.osd {
		animation: osd-pop 180ms ease-out;
	}
	@keyframes osd-pop {
		from {
			transform: scale(0.8);
			opacity: 0;
		}
	}
	/* Show the buffering spinner only when a stall lasts, not on every seek. */
	.buffering {
		animation:
			spin 1s linear infinite,
			appear 0.4s step-end;
	}
	@keyframes appear {
		from {
			opacity: 0;
		}
	}
	.volume {
		accent-color: var(--color-gold);
	}
	@media (prefers-reduced-motion: reduce) {
		.osd {
			animation: none;
		}
	}
</style>
