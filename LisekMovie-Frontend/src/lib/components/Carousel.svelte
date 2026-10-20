<!--
  Shared carousel shell for MediaRow and Top10Row: a horizontally-sliding
  track plus edge arrow buttons. Takes its items via a snippet so each row
  keeps its own card markup — this component only owns the sliding
  mechanics.

  Replaces an earlier version driven entirely by the native scroll APIs
  (`scrollLeft`/`scrollTo`/`scroll-snap`). That approach went through three
  attempts and never became reliably smooth on a real desktop browser (see
  project memory's "Faza 4 refinement round 2" for the full story — the
  short version: native scroll-behavior interacts with scroll-snap in ways
  that differ across browsers/machines, and even a manual rAF loop kept
  getting entangled with it). This version sidesteps all of that: the arrow
  buttons move a plain `offset` number, and a CSS `transition: transform`
  animates the track's `translateX` — no scroll container, no scroll-snap,
  no native smooth-scroll API involved at all. `transition: transform` is
  about as battle-tested as CSS gets (it's how most JS carousel libraries
  animate), so there's no browser-specific smooth-scroll behavior left to
  fight.

  Trackpad/mouse-wheel horizontal panning (`deltaX`) still nudges the
  offset directly, and touch/pointer dragging repositions the track
  1:1 with the finger (transition disabled while dragging, so it doesn't
  lag behind). Vertical wheel scrolling (`deltaY`) is deliberately left
  alone here — it already reaches the page correctly (see MediaRow's and
  Top10Row's own `overflow-y-hidden` note from the previous round); this
  component doesn't re-implement or repeat that fix, it just doesn't
  interfere with it since there's no `overflow-x-auto` container to
  implicitly gain `overflow-y: auto` from anymore.

  Drag engagement is threshold-based (`DRAG_THRESHOLD_PX`), not immediate
  on press — found and fixed after the cards inside this track (once made
  clickable links) turned out to never actually navigate. Capturing the
  pointer on every `pointerdown`, as the first version did, retargets the
  browser's synthesized `click` for the rest of that gesture to the
  capturing element per the Pointer Events spec — so the `click` never
  reached a card's own `<a>` at all, on every single press. See the
  `onPointerDown`/`onPointerMove`/`endDrag` comments below for the fix.

  Faza 4 (TV): the row is a spatial-navigation group (`navGroup`):
  left/right stay inside it, and coming back into the row lands on the
  card it was left on. Focus slides the track (`onFocusIn`) so a card
  reached by keyboard or remote is always fully in view, and focused cards
  scale up inside the vertical padding the viewport keeps for them.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { clampOffset, computeMaxOffset, pageOffset, revealOffset } from '$lib/carouselMath';
	import { navGroup } from '$lib/spatial/spatial';
	import Icon from './Icon.svelte';
	import type { Snippet } from 'svelte';

	interface Props {
		children: Snippet;
	}

	let { children }: Props = $props();

	/** How long the arrow-button slide animates — 0.3s-0.5s reads as a real
	 * glide without feeling sluggish; 380ms sits in the middle of that. */
	const SLIDE_DURATION_MS = 380;

	let viewportEl: HTMLDivElement | undefined = $state();
	let trackEl: HTMLDivElement | undefined = $state();
	let offset = $state(0);
	let maxOffset = $state(0);
	let dragging = $state(false);

	let atStart = $derived(offset <= 0);
	let atEnd = $derived(offset >= maxOffset);

	/** The viewport's width minus its side padding: what the track shows. */
	function visibleWidth(): number {
		if (!viewportEl) return 0;
		const style = getComputedStyle(viewportEl);
		return viewportEl.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
	}

	function recomputeMaxOffset() {
		if (!viewportEl || !trackEl) return;
		// Against the padded width, so at the end the last card stops at the
		// right padding instead of being cut off by the viewport's edge.
		maxOffset = computeMaxOffset(trackEl.scrollWidth, visibleWidth());
		offset = clampOffset(offset, maxOffset);
	}

	/** Keyboard / remote focus: slide the focused card into view. */
	function onFocusIn(event: FocusEvent) {
		if (!viewportEl || !trackEl || !(event.target instanceof HTMLElement)) return;
		// Tab focus may scroll the clipped viewport natively; the track's
		// transform is the only scroll position this carousel uses.
		viewportEl.scrollLeft = 0;
		let card: HTMLElement | null = event.target;
		while (card && card.parentElement !== trackEl) card = card.parentElement;
		if (!card) return;
		const track = trackEl.getBoundingClientRect();
		const rect = card.getBoundingClientRect();
		const start = rect.left - track.left;
		offset = revealOffset(offset, start, start + rect.width, visibleWidth(), maxOffset);
	}

	function scrollByPage(direction: 1 | -1) {
		offset = pageOffset(offset, direction, viewportEl?.clientWidth ?? 0, maxOffset);
	}

	function onWheel(event: WheelEvent) {
		// Only horizontal wheel/trackpad input is ours to handle — vertical
		// wheel scrolling must keep reaching the page untouched.
		if (event.deltaX === 0) return;
		const next = clampOffset(offset + event.deltaX, maxOffset);
		if (next === offset) return; // already at this edge — let it bubble
		event.preventDefault();
		offset = next;
	}

	let dragPointerId: number | null = null;
	let dragStartX = 0;
	let dragStartOffset = 0;
	/** Whether this pointer's movement has actually crossed `DRAG_THRESHOLD_PX`
	 * and engaged real dragging (`setPointerCapture` called) — see the
	 * `onPointerDown` comment below for why capture is deliberately NOT
	 * engaged immediately on press. */
	let dragEngaged = false;

	/** Minimum horizontal movement (CSS px) before a press counts as a drag.
	 * Below this, pointerup is left completely alone and behaves like an
	 * ordinary click/tap on whatever it landed on. */
	const DRAG_THRESHOLD_PX = 6;

	function onPointerDown(event: PointerEvent) {
		if (event.pointerType === 'mouse' && event.button !== 0) return;
		if (maxOffset <= 0) return;
		dragPointerId = event.pointerId;
		dragStartX = event.clientX;
		dragStartOffset = offset;
		dragEngaged = false;
		// Deliberately NOT calling `setPointerCapture` here, and NOT setting
		// `dragging = true` yet. The original version did both immediately on
		// every press — but per the Pointer Events spec, once a pointer is
		// captured, every later event for it (pointermove, pointerup, and the
		// *synthesized `click`* that follows) is retargeted to the capturing
		// element instead of whatever was actually under the pointer. That
		// silently ate the `click` on every card link inside the track: each
		// card's own `<a href>` was wired up correctly, the event just never
		// reached it, on every plain click, every time — capture engaged
		// before the pointer ever moved. Waiting for real movement (see
		// `onPointerMove`) before capturing lets a plain tap/click reach its
		// target completely untouched, while a real drag still works exactly
		// as before once it crosses the threshold.
	}

	function onPointerMove(event: PointerEvent) {
		if (dragPointerId === null || event.pointerId !== dragPointerId) return;
		const deltaX = event.clientX - dragStartX;
		if (!dragEngaged) {
			if (Math.abs(deltaX) < DRAG_THRESHOLD_PX) return;
			dragEngaged = true;
			dragging = true;
			(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
		}
		offset = clampOffset(dragStartOffset - deltaX, maxOffset);
	}

	function endDrag(event: PointerEvent) {
		if (event.pointerId !== dragPointerId) return;
		if (dragEngaged) {
			// A real drag happened, not a plain click — but the browser still
			// fires one `click` right after this pointerup, on whatever card
			// happens to be under the pointer at release. Swallow exactly
			// that one so letting go after a drag never also navigates a
			// card, the same way releasing a native scroll container doesn't
			// "click" whatever you happened to let go over.
			swallowNextClick(event.currentTarget as HTMLElement);
		}
		dragging = false;
		dragEngaged = false;
		dragPointerId = null;
	}

	function swallowNextClick(target: HTMLElement) {
		const suppressOnce = (clickEvent: MouseEvent) => {
			clickEvent.preventDefault();
			clickEvent.stopPropagation();
		};
		target.addEventListener('click', suppressOnce, { capture: true, once: true });
	}

	$effect(() => {
		recomputeMaxOffset();
		if (!viewportEl || !trackEl) return;
		const observer = new ResizeObserver(() => recomputeMaxOffset());
		observer.observe(viewportEl);
		observer.observe(trackEl);
		return () => observer.disconnect();
	});
</script>

<div class="relative -my-4" use:navGroup={{ contain: 'horizontal' }}>
	<!--
	  role="presentation": the pointer handlers below are a drag-to-scroll
	  convenience layered on top of the fully keyboard-accessible arrow
	  buttons, not this element's own interactive semantics — there's
	  nothing here for a screen reader or keyboard user to interact with
	  directly, so this isn't an accessibility regression to route around,
	  just a deliberate "no role of its own" declaration.
	-->
	<div
		bind:this={viewportEl}
		role="presentation"
		class="touch-pan-y overflow-hidden px-4 py-4 md:px-8"
		onwheel={onWheel}
		onfocusin={onFocusIn}
		onpointerdown={onPointerDown}
		onpointermove={onPointerMove}
		onpointerup={endDrag}
		onpointercancel={endDrag}
	>
		<div
			bind:this={trackEl}
			class="flex w-max gap-2 {dragging ? '' : 'transition-transform ease-out'}"
			style="transform: translateX(-{offset}px); {dragging
				? ''
				: `transition-duration: ${SLIDE_DURATION_MS}ms;`}"
		>
			{@render children()}
		</div>
	</div>

	<button
		type="button"
		aria-label={t('home.scroll_left')}
		data-nav-skip
		onclick={() => scrollByPage(-1)}
		class="absolute inset-y-0 left-0 hidden w-10 items-center justify-center bg-gradient-to-r from-base/90 via-base/50 to-transparent text-neutral-100 transition-opacity hover:text-gold md:flex md:w-14 {atStart
			? 'invisible'
			: 'opacity-0 group-hover/row:opacity-100'}"
	>
		<Icon name="chevron_left" class="size-8 transition-transform hover:scale-125" />
	</button>

	<button
		type="button"
		aria-label={t('home.scroll_right')}
		data-nav-skip
		onclick={() => scrollByPage(1)}
		class="absolute inset-y-0 right-0 hidden w-10 items-center justify-center bg-gradient-to-l from-base/90 via-base/50 to-transparent text-neutral-100 transition-opacity hover:text-gold md:flex md:w-14 {atEnd
			? 'invisible'
			: 'opacity-0 group-hover/row:opacity-100'}"
	>
		<Icon name="chevron_right" class="size-8 transition-transform hover:scale-125" />
	</button>
</div>
