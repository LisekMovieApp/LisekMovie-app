import { describe, expect, it } from 'vitest';
import { PLAY_TONE, TOGGLE_SHAPE, toggleTone, type ToggleSurface } from './toggleTone';

/** Colour utilities per variant (`''`, `hover:`, `focus-visible:`):
 * `text-<colour>` and `bg-<colour>`, not sizes like `text-sm`. */
function colours(classes: string): Record<string, string[]> {
	const out: Record<string, string[]> = {};
	for (const token of classes.split(/\s+/).filter(Boolean)) {
		const match = token.match(/^((?:[\w-]+:)*)(text|bg)-(.+)$/);
		if (!match) continue;
		const [, variant, kind, value] = match;
		if (kind === 'text' && /^(xs|sm|base|lg|xl|\d?xl|\[\d)/.test(value)) continue;
		(out[`${variant}${kind}`] ??= []).push(value);
	}
	return out;
}

describe('state button tones', () => {
	const surfaces: ToggleSurface[] = ['plain', 'subtle', 'overlay'];

	it('never puts two colours of one kind on a button', () => {
		for (const surface of surfaces) {
			for (const kind of ['watched', 'favorite'] as const) {
				for (const active of [false, true]) {
					const found = colours(`${TOGGLE_SHAPE} size-11 ${toggleTone(kind, active, surface)}`);
					for (const [slot, values] of Object.entries(found)) {
						expect(values, `${kind} ${active} ${surface} ${slot}`).toHaveLength(1);
					}
				}
			}
		}
		for (const [slot, values] of Object.entries(colours(`${TOGGLE_SHAPE} ${PLAY_TONE}`))) {
			expect(values, slot).toHaveLength(1);
		}
	});

	it('keeps colours out of the shared shape', () => {
		expect(Object.keys(colours(TOGGLE_SHAPE))).toEqual([]);
	});

	it('shows a watched title as a gold disc with a dark check', () => {
		const watched = colours(toggleTone('watched', true));
		expect(watched.bg).toEqual(['gold']);
		expect(watched.text).toEqual(['black']);
		expect(colours(toggleTone('watched', false)).text).toEqual(['neutral-100']);
		expect(colours(toggleTone('favorite', true)).text).toEqual(['red-500']);
	});
});
