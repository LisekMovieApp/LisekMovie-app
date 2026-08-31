import { describe, it, expect } from 'vitest';
import { translate } from './translations';

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
});
