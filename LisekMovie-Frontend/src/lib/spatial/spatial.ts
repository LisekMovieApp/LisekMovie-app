/**
 * Spatial navigation: arrow keys and a TV remote's D-pad move focus to the
 * nearest focusable element in that direction, so every page works from
 * the sofa without a mouse.
 *
 * - Installed once (`initSpatialNavigation` in the root layout); every
 *   link, button and `[data-nav]` element takes part automatically.
 * - `use:navGroup` marks a row (carousel) or a grid. A group remembers the
 *   element that last had focus there: coming back into a row lands on
 *   the card you left, not on whichever card happens to be nearest (the
 *   "virtual focus" TV interfaces use). On a first visit it lands on the
 *   group's `[data-nav-default]` when it has one. `contain: 'horizontal'` keeps
 *   left/right inside the row, so its end is a wall, not a jump to the
 *   next row.
 * - `use:navScope` confines navigation to a layer (a dialog, the player's
 *   controls) while it is on screen; the newest scope wins.
 * - Focus moves with `preventScroll` and announces itself with a
 *   `spatialfocus` event, so carousels slide the card into view instead
 *   of the browser scrolling their clipped viewport. The page then
 *   scrolls vertically to keep the focused row in view.
 * - The remote's Back key dispatches a cancellable `spatialback` event on
 *   `window`; if nothing handles it, the browser goes back.
 *
 * Components that use arrows themselves (sliders, the player while its
 * controls are hidden) call `preventDefault()` and this module stays out.
 */

import { directionOf, isBackKey, pickNext, type Direction } from './geometry';

const FOCUSABLE = [
	'a[href]',
	'button:not([disabled])',
	'input:not([disabled]):not([type="hidden"])',
	'select:not([disabled])',
	'textarea:not([disabled])',
	'[tabindex]:not([tabindex="-1"])',
	'[data-nav]'
].join(',');

export interface GroupOptions {
	/** Return to the last focused element when re-entering (default on). */
	memory?: boolean;
	/** Keep movement along this axis inside the group. */
	contain?: 'horizontal' | 'vertical' | false;
}

const remembered = new WeakMap<HTMLElement, HTMLElement>();
const scopes: HTMLElement[] = [];

function isVisible(el: HTMLElement): boolean {
	if (el.closest('[inert], [hidden], [aria-hidden="true"]')) return false;
	const rect = el.getBoundingClientRect();
	if (rect.width === 0 && rect.height === 0) return false;
	const style = getComputedStyle(el);
	return style.visibility !== 'hidden' && style.display !== 'none';
}

/** The layer navigation is confined to: the newest scope on screen. */
export function activeScope(): ParentNode {
	for (let i = scopes.length - 1; i >= 0; i--) {
		if (scopes[i].isConnected && isVisible(scopes[i])) return scopes[i];
	}
	return document.body;
}

export function focusables(scope: ParentNode = activeScope()): HTMLElement[] {
	return Array.from(scope.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
		(el) => !el.hasAttribute('data-nav-skip') && isVisible(el)
	);
}

/** Moves focus without the browser scrolling, then reveals the element. */
export function focusElement(el: HTMLElement) {
	el.focus({ preventScroll: true });
	el.dispatchEvent(new CustomEvent('spatialfocus', { bubbles: true }));
	reveal(el);
}

const REVEAL_MARGIN = 96;

/** Scrolls the page so the element (or its whole row) is comfortably visible. */
function reveal(el: HTMLElement) {
	const target = el.closest<HTMLElement>('[data-nav-group]') ?? el;
	const rect = target.getBoundingClientRect();
	const viewport = window.innerHeight;
	if (rect.top >= REVEAL_MARGIN && rect.bottom <= viewport - REVEAL_MARGIN / 2) return;
	const reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
	// Rows sit a third of the way down, like TV interfaces; tall content
	// aligns its top.
	const offset = rect.height > viewport * 0.6 ? rect.top - REVEAL_MARGIN : rect.top - viewport / 3;
	window.scrollBy({ top: offset, behavior: reduced ? 'auto' : 'smooth' });
}

function groupOf(el: Element | null): HTMLElement | null {
	return el?.closest<HTMLElement>('[data-nav-group]') ?? null;
}

/**
 * Where focus should land when entering `group`: the element remembered
 * from the last visit, else the group's `[data-nav-default]` (a hero's
 * Play button, the selected option in a list), else the nearest one.
 */
function entryPoint(group: HTMLElement | null, fallback: HTMLElement): HTMLElement {
	if (!group || group.dataset.navMemory === 'off') return fallback;
	const last = remembered.get(group);
	if (last && last.isConnected && group.contains(last) && isVisible(last)) return last;
	const preferred = group.querySelector<HTMLElement>('[data-nav-default]');
	return preferred && isVisible(preferred) && !preferred.hasAttribute('data-nav-skip')
		? preferred
		: fallback;
}

/** Moves focus one step; `false` when there is nowhere to go. */
export function moveFocus(direction: Direction): boolean {
	const scope = activeScope();
	const all = focusables(scope);
	const current = document.activeElement as HTMLElement | null;
	if (!current || current === document.body || !scope.contains(current)) {
		const start =
			scope.querySelector<HTMLElement>('[data-nav-default]') ??
			all.find((el) => {
				const r = el.getBoundingClientRect();
				return r.bottom > 0 && r.top < window.innerHeight;
			}) ??
			all[0];
		if (!start) return false;
		focusElement(entryPoint(groupOf(start), start));
		return true;
	}
	const group = groupOf(current);
	const horizontal = direction === 'left' || direction === 'right';
	const contained =
		group &&
		((horizontal && group.dataset.navContain === 'horizontal') ||
			(!horizontal && group.dataset.navContain === 'vertical'));
	const pool = all.filter((el) => el !== current && (!contained || group.contains(el)));
	const next = pickNext(
		current.getBoundingClientRect(),
		pool.map((el) => ({ item: el, rect: el.getBoundingClientRect() })),
		direction
	);
	if (!next) return false;
	const nextGroup = groupOf(next);
	focusElement(nextGroup && nextGroup !== group ? entryPoint(nextGroup, next) : next);
	return true;
}

function typesText(el: Element | null): boolean {
	if (!el) return false;
	if (el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return true;
	if (el instanceof HTMLElement && el.isContentEditable) return true;
	if (el instanceof HTMLInputElement) {
		return !['checkbox', 'radio', 'button', 'submit', 'reset', 'range', 'color', 'file'].includes(
			el.type
		);
	}
	return false;
}

function onKeyDown(event: KeyboardEvent) {
	if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
	const target = event.target as Element | null;
	if (isBackKey(event)) {
		const handled = !window.dispatchEvent(new CustomEvent('spatialback', { cancelable: true }));
		event.preventDefault();
		if (!handled && history.length > 1) history.back();
		return;
	}
	const direction = directionOf(event);
	if (direction) {
		// Text fields keep left/right for the caret; up/down leave them.
		if (typesText(target) && (direction === 'left' || direction === 'right')) return;
		if (target instanceof HTMLInputElement && target.type === 'range') return;
		if (moveFocus(direction)) event.preventDefault();
		return;
	}
	if (
		event.key === 'Enter' &&
		target instanceof HTMLElement &&
		target.matches('[data-nav]:not(a,button,input,select,textarea)')
	) {
		event.preventDefault();
		target.click();
	}
}

/** Installs the key handler; returns the uninstaller. */
export function initSpatialNavigation(): () => void {
	window.addEventListener('keydown', onKeyDown);
	return () => window.removeEventListener('keydown', onKeyDown);
}

function applyGroup(node: HTMLElement, options: GroupOptions) {
	node.dataset.navGroup = '';
	node.dataset.navMemory = options.memory === false ? 'off' : 'on';
	if (options.contain) node.dataset.navContain = options.contain;
	else delete node.dataset.navContain;
}

/** Svelte action: a row or grid that remembers its last focused element. */
export function navGroup(node: HTMLElement, options: GroupOptions = {}) {
	applyGroup(node, options);
	const onFocusIn = (event: FocusEvent) => {
		if (event.target instanceof HTMLElement) remembered.set(node, event.target);
	};
	node.addEventListener('focusin', onFocusIn);
	return {
		update(next: GroupOptions = {}) {
			applyGroup(node, next);
		},
		destroy() {
			node.removeEventListener('focusin', onFocusIn);
		}
	};
}

/**
 * Svelte action: confines arrow navigation to `node` while it is on
 * screen (dialogs, the player). `autofocus` focuses its
 * `[data-nav-default]` element (or first focusable) when it appears.
 */
export function navScope(node: HTMLElement, options: { autofocus?: boolean } = {}) {
	node.dataset.navScope = '';
	scopes.push(node);
	if (options.autofocus) {
		queueMicrotask(() => {
			const first = node.querySelector<HTMLElement>('[data-nav-default]') ?? focusables(node)[0];
			first?.focus({ preventScroll: true });
		});
	}
	return {
		destroy() {
			const index = scopes.indexOf(node);
			if (index >= 0) scopes.splice(index, 1);
		}
	};
}
