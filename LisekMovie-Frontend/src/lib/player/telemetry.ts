/**
 * Playback telemetry from the player: a progress report every 15 s while
 * playing (30 s while paused, as the server's `next_report_seconds`
 * says), plus one on every play, pause, seek, end and exit, so the resume
 * point is never more than a few seconds stale. Each report also carries
 * rebuffering (stalls that are not seeks), dropped frames and the
 * bandwidth estimate.
 */
import type { components } from '$lib/api/schema';

export type PlaybackProgress = components['schemas']['PlaybackProgress'];
export type ProgressAck = components['schemas']['ProgressAck'];
export type ProgressEvent = NonNullable<PlaybackProgress['event']>;

export interface MediaSnapshot {
	currentTime: number;
	duration: number;
	paused: boolean;
	bufferedSeconds?: number;
	droppedFrames?: number;
	bandwidthKbps?: number;
}

export interface ReporterOptions {
	sessionId: string;
	/** Duration the server knows (HLS playlists, probes) when the media element does not yet. */
	durationHint?: number | null;
	snapshot: () => MediaSnapshot;
	/** Delivers a report; `final` reports must survive the page closing. */
	send: (report: PlaybackProgress, final: boolean) => Promise<ProgressAck | undefined>;
	onDelivery?: (ok: boolean) => void;
	now?: () => number;
}

const MIN_INTERVAL = 10;
const MAX_INTERVAL = 30;

export class ProgressReporter {
	#options: ReporterOptions;
	#timer: ReturnType<typeof setTimeout> | undefined;
	#queue: Promise<unknown> = Promise.resolve();
	#interval = 15;
	#stalls = 0;
	#stallSeconds = 0;
	#stallStart: number | null = null;
	#seeking = false;
	#stopped = false;

	constructor(options: ReporterOptions) {
		this.#options = options;
	}

	get stopped() {
		return this.#stopped;
	}

	#now() {
		return (this.#options.now ?? (() => performance.now()))();
	}

	start() {
		this.#schedule();
	}

	#schedule() {
		clearTimeout(this.#timer);
		if (this.#stopped) return;
		this.#timer = setTimeout(() => this.report('heartbeat'), this.#interval * 1000);
	}

	/** Media element hooks. */
	onPlay = () => this.report('play');
	onPause = () => {
		// `pause` also fires right before `ended`; the end report covers it.
		const { currentTime, duration } = this.#options.snapshot();
		if (!(duration > 0 && currentTime >= duration - 0.5)) this.report('pause');
	};
	onSeeking = () => {
		this.#seeking = true;
		this.#stallStart = null;
	};
	onSeeked = () => {
		this.#seeking = false;
		this.report('seek');
	};
	onWaiting = () => {
		if (!this.#seeking && this.#stallStart === null) this.#stallStart = this.#now();
	};
	onPlaying = () => {
		if (this.#stallStart !== null) {
			this.#stalls += 1;
			this.#stallSeconds += (this.#now() - this.#stallStart) / 1000;
			this.#stallStart = null;
		}
	};
	onEnded = () => this.stop('ended');

	#build(event: ProgressEvent): PlaybackProgress {
		const media = this.#options.snapshot();
		const duration =
			media.duration > 0 && Number.isFinite(media.duration)
				? media.duration
				: (this.#options.durationHint ?? 0);
		const position = Math.max(
			0,
			Math.min(media.currentTime || 0, duration > 0 ? duration : Infinity)
		);
		const report: PlaybackProgress = {
			session_id: this.#options.sessionId,
			position_seconds: position,
			duration_seconds: duration,
			paused: media.paused,
			event,
			stalls: this.#stalls,
			stall_seconds: Math.round(this.#stallSeconds * 1000) / 1000,
			dropped_frames: media.droppedFrames ?? null,
			bandwidth_kbps: media.bandwidthKbps ? Math.round(media.bandwidthKbps) : null,
			buffered_seconds: media.bufferedSeconds ?? null
		};
		this.#stalls = 0;
		this.#stallSeconds = 0;
		return report;
	}

	/** Sends a report now; reports are delivered one at a time, in order. */
	report(event: ProgressEvent, final = false): Promise<unknown> {
		if (this.#stopped && !final) return this.#queue;
		const report = this.#build(event);
		this.#queue = this.#queue
			.catch(() => undefined)
			.then(() => this.#options.send(report, final))
			.then((ack) => {
				if (ack) {
					this.#interval = Math.min(MAX_INTERVAL, Math.max(MIN_INTERVAL, ack.next_report_seconds));
					this.#schedule();
				}
				this.#options.onDelivery?.(true);
			})
			.catch(() => this.#options.onDelivery?.(false));
		this.#schedule();
		return this.#queue;
	}

	/** Final report (`stop` when leaving, `ended` at the end); idempotent. */
	stop(event: 'stop' | 'ended' = 'stop'): Promise<unknown> {
		if (this.#stopped) return this.#queue;
		clearTimeout(this.#timer);
		const sent = this.report(event, true);
		this.#stopped = true;
		return sent;
	}
}
