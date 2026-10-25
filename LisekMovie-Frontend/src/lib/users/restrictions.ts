// Per-account access restrictions (Faza 6, parental controls):
// `GET/PUT /api/admin/users/{id}/restrictions`, `GET /api/admin/user-restrictions`.

import type { components } from '$lib/api/schema';
import { accountRequest } from './api';

export type Restrictions = components['schemas']['Restrictions'];
export type UserRestrictions = components['schemas']['UserRestrictionsResponse'];
export type RestrictedUser = components['schemas']['RestrictedUser'];

/** Age limits offered in the editor (minimum viewer age). PG → 10,
 * PG-13 → 13, R → 17, FSK 12 → 12, TV-14 → 14. */
export const AGE_LIMITS = [0, 7, 10, 12, 13, 16, 18] as const;

/** Quick profiles: a young child, a teenager, nobody restricted. */
export const PRESETS = {
	child: { max_age_rating: 7, block_unrated: true },
	teen: { max_age_rating: 13, block_unrated: false },
	none: { max_age_rating: null, block_unrated: false }
} as const satisfies Record<string, Pick<Restrictions, 'max_age_rating' | 'block_unrated'>>;

export function getRestrictions(token: string, userId: string): Promise<UserRestrictions> {
	return accountRequest<UserRestrictions>(token, `/api/admin/users/${userId}/restrictions`);
}

export function saveRestrictions(
	token: string,
	userId: string,
	restrictions: Restrictions
): Promise<UserRestrictions> {
	return accountRequest<UserRestrictions>(
		token,
		`/api/admin/users/${userId}/restrictions`,
		'PUT',
		restrictions
	);
}

export function listRestricted(token: string): Promise<RestrictedUser[]> {
	return accountRequest<RestrictedUser[]>(token, '/api/admin/user-restrictions');
}

/** Whether anything is actually limited. */
export function isRestricted(r: Restrictions): boolean {
	return (r.libraries !== null && r.libraries !== undefined) || (r.max_age_rating ?? null) !== null;
}

/** The editor's form state turned into the request body: "every library"
 * sends `null`, the unrated switch only counts with an age limit. */
export function toRequest(
	allLibraries: boolean,
	libraries: string[],
	maxAge: number | null,
	blockUnrated: boolean
): Restrictions {
	return {
		libraries: allLibraries ? null : [...libraries].sort(),
		max_age_rating: maxAge,
		block_unrated: maxAge !== null && blockUnrated
	};
}
