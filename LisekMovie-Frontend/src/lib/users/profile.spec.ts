import { describe, expect, it } from 'vitest';
import {
	avatarFileError,
	avatarSrc,
	profileErrorKey,
	ProfileError,
	relativeTime,
	MAX_AVATAR_BYTES
} from './profile';

describe('profile helpers', () => {
	it('adds the session to avatar URLs for <img>', () => {
		expect(avatarSrc(null, 't')).toBeNull();
		expect(avatarSrc('/api/users/1/avatar?v=5', null)).toBe('/api/users/1/avatar?v=5');
		expect(avatarSrc('/api/users/1/avatar?v=5', 'a b')).toBe('/api/users/1/avatar?v=5&token=a%20b');
		expect(avatarSrc('/api/users/1/avatar', 'x')).toBe('/api/users/1/avatar?token=x');
	});

	it('checks picture type and size before uploading', () => {
		expect(avatarFileError({ type: 'image/png', size: 1000 })).toBeNull();
		expect(avatarFileError({ type: 'image/svg+xml', size: 1000 })).toBe(
			'profile.errors.avatar_invalid_image'
		);
		expect(avatarFileError({ type: 'image/jpeg', size: MAX_AVATAR_BYTES + 1 })).toBe(
			'profile.errors.avatar_too_large'
		);
		expect(avatarFileError({ type: 'image/webp', size: 0 })).toBe('profile.errors.avatar_empty');
	});

	it('maps server error codes to translation keys', () => {
		expect(profileErrorKey(new ProfileError('avatar_too_large'))).toBe(
			'profile.errors.avatar_too_large'
		);
		expect(profileErrorKey(new Error('something_else'))).toBe('profile.errors.operation_failed');
		expect(profileErrorKey('boom')).toBe('profile.errors.operation_failed');
	});

	it('formats "last seen" in the UI language', () => {
		const now = Date.parse('2026-10-28T12:00:00Z');
		expect(relativeTime('2026-10-28T11:55:00Z', 'en', now)).toBe('5 minutes ago');
		expect(relativeTime('2026-10-28T09:00:00Z', 'pl', now)).toBe('3 godziny temu');
		expect(relativeTime('2026-10-27T12:00:00Z', 'en', now)).toBe('yesterday');
	});
});
