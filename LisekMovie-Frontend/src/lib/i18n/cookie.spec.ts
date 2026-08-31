import { describe, it, expect } from 'vitest';
import { parseLocaleCookie } from './cookie';

describe('parseLocaleCookie', () => {
	it('accepts a supported locale', () => {
		expect(parseLocaleCookie('pl')).toBe('pl');
		expect(parseLocaleCookie('en')).toBe('en');
	});

	it('rejects an unsupported or malformed value', () => {
		expect(parseLocaleCookie('cs')).toBeNull();
		expect(parseLocaleCookie('not-a-locale')).toBeNull();
		expect(parseLocaleCookie('')).toBeNull();
	});

	it('treats a missing cookie as no preference', () => {
		expect(parseLocaleCookie(undefined)).toBeNull();
	});
});
