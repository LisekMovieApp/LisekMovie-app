import { describe, it, expect } from 'vitest';
import { translate } from './translations';
import en from './en.json';
import pl from './pl.json';

function flattenKeys(node: unknown, prefix = ''): string[] {
	if (typeof node !== 'object' || node === null) return [prefix];
	return Object.entries(node as Record<string, unknown>).flatMap(([key, value]) =>
		flattenKeys(value, prefix ? `${prefix}.${key}` : key)
	);
}

describe('translate', () => {
	it('resolves a nested key for the requested locale', () => {
		expect(translate('pl', 'common.next')).toBe('Dalej');
		expect(translate('en', 'common.next')).toBe('Next');
	});

	it('substitutes {name} placeholders', () => {
		expect(translate('en', 'setup.step_indicator', { current: 2, total: 3 })).toBe('Step 2 of 3');
	});

	it('falls back to the raw key when nothing matches, in any locale', () => {
		expect(translate('en', 'nonexistent.key')).toBe('nonexistent.key');
	});

	it('en.json and pl.json define exactly the same set of keys', () => {
		// Catches a translation added to only one dictionary — the fallback to
		// DEFAULT_LOCALE would hide the gap silently in the running app.
		const enKeys = flattenKeys(en).sort();
		const plKeys = flattenKeys(pl).sort();
		expect(plKeys).toEqual(enKeys);
	});
});
