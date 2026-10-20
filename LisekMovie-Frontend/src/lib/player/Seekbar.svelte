<!--
  The player's timeline. Dragging, clicking and the keyboard only set a
  pending target (shown as a ghost bar and a time bubble); the player
  seeks once, on release or a moment after the last key press, so fast
  seeking does not restart the server's encoder for every step.

  With trickplay sheets (`thumbnail`), the bubble becomes a picture of the
  moment under the pointer, the drag or the pending keyboard jump, so a
  remote user sees where a seek lands before it happens.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { formatTime } from './format';
	import type { Thumbnail } from './trickplay';

	interface Props {
		currentTime: number;
		duration: number;
		bufferedTo: number;
		/** Target of a seek in progress (keyboard or drag), or null. */
		pending: number | null;
		/** Moves the pending target to `time`; `scrubbing` while the pointer is down. */
		onscrub: (time: number, scrubbing: boolean) => void;
		/** Seeks to the pending target now. */
		oncommit: () => void;
		/** Keyboard step in seconds (negative = back); `repeat` while a key is held. */
		onstep: (seconds: number, repeat: boolean) => void;
		/** OK / Enter / Space on the focused bar. */
		ontoggle: () => void;
		/** Focused first when the controls appear (TV remotes). */
		navDefault?: boolean;
		/** Preview tile for a moment, when the item has trickplay sheets. */
		thumbnail?: ((time: number) => Thumbnail | null) | null;
		/** The bar is about to be used (hover, focus): preload the sheets. */
		onwarm?: () => void;
	}

	let {
		currentTime,
		duration,
		bufferedTo,
		pending,
		onscrub,
		oncommit,
		onstep,
		ontoggle,
		navDefault = false,
		thumbnail = null,
		onwarm
	}: Props = $props();

	let bar = $state<HTMLDivElement>();
	let hover = $state<number | null>(null);
	let dragging = $state(false);

	const known = $derived(Number.isFinite(duration) && duration > 0);
	const percent = (time: number) =>
		known ? Math.min(100, Math.max(0, (time / duration) * 100)) : 0;
	const shown = $derived(pending ?? currentTime);
	const bubble = $derived(dragging || pending !== null ? shown : hover);
	const preview = $derived(bubble !== null && thumbnail ? thumbnail(bubble) : null);
	/** Preview width on screen: larger on big screens (TVs), never upscaled much. */
	let previewWidth = $state(240);
	const previewScale = $derived(preview ? previewWidth / preview.width : 1);

	function measure() {
		previewWidth = Math.round(Math.min(320, Math.max(160, window.innerWidth * 0.15)));
	}

	function timeAt(clientX: number): number {
		if (!bar || !known) return 0;
		const rect = bar.getBoundingClientRect();
		return Math.min(1, Math.max(0, (clientX - rect.left) / rect.width)) * duration;
	}

	function onPointerDown(event: PointerEvent) {
		if (!known || event.button !== 0) return;
		bar?.setPointerCapture(event.pointerId);
		dragging = true;
		onscrub(timeAt(event.clientX), true);
	}

	function onPointerMove(event: PointerEvent) {
		const time = timeAt(event.clientX);
		if (event.pointerType === 'mouse') hover = time;
		if (dragging) onscrub(time, true);
	}

	function onPointerUp(event: PointerEvent) {
		if (!dragging) return;
		dragging = false;
		bar?.releasePointerCapture(event.pointerId);
		onscrub(timeAt(event.clientX), false);
		oncommit();
	}

	function onKeyDown(event: KeyboardEvent) {
		// The player's own shortcuts (Space, k, j, l) already ran.
		if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
		const steps: Record<string, number> = {
			ArrowLeft: -10,
			ArrowRight: 10,
			PageDown: -60,
			PageUp: 60
		};
		if (event.key in steps) {
			event.preventDefault();
			onstep(steps[event.key], event.repeat);
		} else if (event.key === 'Home' || event.key === 'End') {
			event.preventDefault();
			onscrub(event.key === 'Home' ? 0 : Math.max(0, duration - 1), false);
			oncommit();
		} else if (event.key === 'Enter' || event.key === ' ') {
			event.preventDefault();
			ontoggle();
		}
	}
</script>

<div
	bind:this={bar}
	class="group/seek relative flex h-6 w-full cursor-pointer touch-none items-center outline-none select-none"
	role="slider"
	tabindex="0"
	aria-label={t('player.seek')}
	aria-valuemin={0}
	aria-valuemax={Math.round(known ? duration : 0)}
	aria-valuenow={Math.round(shown)}
	aria-valuetext={t('player.time_of', { time: formatTime(shown), total: formatTime(duration) })}
	data-nav-default={navDefault ? '' : undefined}
	onpointerdown={onPointerDown}
	onpointermove={onPointerMove}
	onpointerup={onPointerUp}
	onpointercancel={onPointerUp}
	onpointerleave={() => (hover = null)}
	onpointerenter={() => {
		measure();
		onwarm?.();
	}}
	onfocus={() => {
		measure();
		onwarm?.();
	}}
	onkeydown={onKeyDown}
>
	<div
		class="relative h-1 w-full overflow-hidden rounded-full bg-white/20 transition-[height] duration-150 group-hover/seek:h-1.5 group-focus-visible/seek:h-2"
	>
		<div class="absolute inset-y-0 left-0 bg-white/35" style:width="{percent(bufferedTo)}%"></div>
		{#if hover !== null && !dragging}
			<div class="absolute inset-y-0 left-0 bg-white/25" style:width="{percent(hover)}%"></div>
		{/if}
		<div class="absolute inset-y-0 left-0 bg-gold" style:width="{percent(shown)}%"></div>
		{#if pending !== null}
			<!-- Where playback is now while a jump is pending. -->
			<div class="absolute inset-y-0 w-0.5 bg-white" style:left="{percent(currentTime)}%"></div>
		{/if}
	</div>
	<div
		class="pointer-events-none absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 scale-0 rounded-full bg-gold shadow-[0_0_0_4px_rgb(189_162_77/0.3)] transition-transform duration-150 group-hover/seek:scale-100 group-focus-visible/seek:scale-125"
		class:scale-100={dragging || pending !== null}
		style:left="{percent(shown)}%"
	></div>
	{#if bubble !== null && known && preview}
		{@const w = previewWidth}
		{@const h = Math.round(preview.height * previewScale)}
		<div
			class="pointer-events-none absolute bottom-full mb-4 flex -translate-x-1/2 flex-col items-center gap-1.5"
			style:left={`clamp(${w / 2}px, ${percent(bubble)}%, calc(100% - ${w / 2}px))`}
		>
			<div
				class="overflow-hidden rounded-md bg-black bg-no-repeat shadow-[0_8px_30px_rgb(0_0_0/0.7)] ring-2 ring-white/80"
				style:width={`${w}px`}
				style:height={`${h}px`}
				style:background-image={`url("${preview.url}")`}
				style:background-size={`${preview.sheetWidth * previewScale}px ${preview.sheetHeight * previewScale}px`}
				style:background-position={`-${preview.x * previewScale}px -${preview.y * previewScale}px`}
			></div>
			<span class="rounded bg-black/85 px-2 py-0.5 text-sm font-semibold text-white tabular-nums"
				>{formatTime(bubble)}</span
			>
		</div>
	{:else if bubble !== null && known}
		<div
			class="pointer-events-none absolute bottom-full mb-3 -translate-x-1/2 rounded-md bg-black/85 px-2 py-1 text-sm font-semibold text-white tabular-nums ring-1 ring-white/15"
			style:left={`clamp(1.75rem, ${percent(bubble)}%, calc(100% - 1.75rem))`}
		>
			{formatTime(bubble)}
		</div>
	{/if}
</div>

<style>
	/* The global focus ring would outline the whole hit area; the bar itself grows instead. */
	div[role='slider']:focus-visible {
		outline: none;
	}
</style>
