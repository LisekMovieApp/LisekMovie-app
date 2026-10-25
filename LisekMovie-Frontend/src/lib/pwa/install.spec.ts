import { describe, expect, it } from 'vitest';
import { isIos } from './install.svelte';

describe('isIos', () => {
	it('recognizes iPhones and iPads, including desktop-mode iPads', () => {
		expect(isIos('Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)', 5)).toBe(true);
		expect(isIos('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', 5)).toBe(true);
		expect(isIos('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)', 0)).toBe(false);
		expect(isIos('Mozilla/5.0 (Linux; Android 15; Pixel 9)', 5)).toBe(false);
	});
});
