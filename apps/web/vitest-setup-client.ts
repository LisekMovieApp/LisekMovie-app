// Setup for the "client" (jsdom) vitest project — see `vite.config.ts`.
// Runda 10: this project's first Svelte-component-mounting tests, so this
// file's whole job is filling in the handful of browser APIs jsdom doesn't
// implement that this app's components happen to touch on mount.

import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/svelte';

// Unmounts whatever the previous test rendered — without this, two tests
// in the same file would both have their component trees live in the same
// jsdom `document` at once, and `screen.getByText(...)`-style queries would
// start throwing "found multiple elements" instead of the real assertion
// failure.
afterEach(() => cleanup());

// jsdom has no ResizeObserver at all. `Carousel.svelte` (rendered inside
// every `MediaRow`, which every regression test in this project mounts)
// creates one in its own `$effect` to measure its scroll width — this test
// suite cares about reactive state and rendered DOM text, never about real
// layout/resize behavior, so a no-op stub is enough to let that `$effect`
// run without throwing.
class ResizeObserverStub {
	observe() {}
	unobserve() {}
	disconnect() {}
}
// @ts-expect-error test-only global polyfill — jsdom doesn't define this class at all
globalThis.ResizeObserver = ResizeObserverStub;

// Runda 11: jsdom has no Web Animations API either — `Element.animate` is
// entirely missing — which `Navbar.svelte.test.ts` hit for the first time
// in this project, because it's the first test to mount a component using
// `transition:fade` (each Navbar library link). Svelte's own transition
// runtime calls `element.animate(...)`, then assigns `.onfinish` to chain a
// short "dummy delay" animation into the real one; without a real browser
// there's nothing to ever call that callback, so it's invoked here on the
// next microtask — fast enough that no test needs to wait for a fade to
// "complete", but real enough that Svelte's own transition state machine
// runs to completion instead of stalling mid-transition across tests.
class FakeAnimation {
	onfinish: (() => void) | null = null;
	effect: unknown = null;
	playState: 'running' | 'finished' = 'running';
	currentTime = 0;
	cancel() {}
	pause() {}
	play() {}
	finish() {
		this.playState = 'finished';
		this.onfinish?.();
	}
}
Element.prototype.animate = function animateStub() {
	const animation = new FakeAnimation();
	queueMicrotask(() => animation.finish());
	return animation as unknown as Animation;
};
