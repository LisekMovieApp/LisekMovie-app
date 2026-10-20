// DOM behaviour of spatial navigation, with layout faked through
// `getBoundingClientRect` (jsdom has no layout engine).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { initSpatialNavigation, navGroup, navScope } from './spatial';

function place(el: HTMLElement, left: number, top: number, width = 100, height = 60) {
	el.getBoundingClientRect = () =>
		({
			left,
			top,
			right: left + width,
			bottom: top + height,
			width,
			height,
			x: left,
			y: top
		}) as DOMRect;
}

function button(label: string, left: number, top: number, parent: HTMLElement) {
	const el = document.createElement('button');
	el.textContent = label;
	place(el, left, top);
	parent.append(el);
	return el;
}

function press(key: string, keyCode = 0) {
	const event = new KeyboardEvent('keydown', { key, keyCode, bubbles: true, cancelable: true });
	(document.activeElement ?? document.body).dispatchEvent(event);
	return event;
}

let uninstall: () => void;
let rows: HTMLElement[];
let cards: HTMLButtonElement[][];

beforeEach(() => {
	document.body.innerHTML = '';
	window.scrollBy = vi.fn();
	uninstall = initSpatialNavigation();
	const nav = document.createElement('nav');
	document.body.append(nav);
	button('Home', 0, 10, nav);
	rows = [];
	cards = [];
	for (const [r, top] of [200, 320].entries()) {
		const row = document.createElement('section');
		place(row, 0, top, 1000, 100);
		document.body.append(row);
		navGroup(row, { contain: 'horizontal' });
		rows.push(row);
		cards.push([0, 1, 2].map((c) => button(`r${r}c${c}`, c * 120, top + 20, row)));
	}
});

afterEach(() => uninstall());

describe('spatial navigation', () => {
	it('focuses something visible on the first arrow press', () => {
		expect(press('ArrowDown').defaultPrevented).toBe(true);
		expect(document.activeElement?.textContent).toBe('Home');
	});

	it('moves within a row, stops at its end and goes down by geometry', () => {
		cards[0][1].focus();
		press('ArrowRight');
		expect(document.activeElement).toBe(cards[0][2]);
		expect(press('ArrowRight').defaultPrevented).toBe(false);
		expect(document.activeElement, 'contained: no jump to the next row').toBe(cards[0][2]);
		press('ArrowDown');
		expect(document.activeElement).toBe(cards[1][2]);
	});

	it('returns to the card last focused in a row (virtual focus)', () => {
		cards[0][0].focus();
		press('ArrowDown');
		expect(document.activeElement).toBe(cards[1][0]);
		press('ArrowRight');
		press('ArrowRight');
		press('ArrowUp');
		expect(document.activeElement, 'not r0c2, which is straight above').toBe(cards[0][0]);
		press('ArrowUp');
		expect(document.activeElement?.textContent).toBe('Home');
	});

	it('enters a group on its default element when it has no memory yet', () => {
		cards[1][2].dataset.navDefault = '';
		cards[0][0].focus();
		press('ArrowDown');
		expect(document.activeElement, 'the default, not r1c0 straight below').toBe(cards[1][2]);
	});

	it('announces focus so carousels can slide the card into view', () => {
		const seen = vi.fn();
		rows[0].addEventListener('spatialfocus', seen);
		cards[0][0].focus();
		press('ArrowRight');
		expect(seen).toHaveBeenCalledOnce();
	});

	it('confines navigation to the newest scope', () => {
		const dialog = document.createElement('div');
		place(dialog, 300, 300, 400, 300);
		document.body.append(dialog);
		const scope = navScope(dialog);
		const ok = button('OK', 320, 500, dialog);
		const cancel = button('Cancel', 440, 500, dialog);
		ok.focus();
		press('ArrowRight');
		expect(document.activeElement).toBe(cancel);
		press('ArrowUp');
		expect(document.activeElement, 'nothing above inside the dialog').toBe(cancel);
		scope.destroy();
	});

	it('turns the remote Back key into a cancellable spatialback event', () => {
		const back = vi.spyOn(history, 'back');
		const handler = (event: Event) => event.preventDefault();
		window.addEventListener('spatialback', handler);
		press('Unidentified', 10009);
		expect(back).not.toHaveBeenCalled();
		window.removeEventListener('spatialback', handler);
	});

	it('leaves text fields their left/right arrows', () => {
		const input = document.createElement('input');
		place(input, 0, 600);
		document.body.append(input);
		input.focus();
		expect(press('ArrowLeft').defaultPrevented).toBe(false);
		press('ArrowUp');
		expect(document.activeElement).not.toBe(input);
	});
});
