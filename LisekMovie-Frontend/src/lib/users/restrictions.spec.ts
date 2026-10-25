import { describe, expect, it } from 'vitest';
import { isRestricted, toRequest } from './restrictions';

describe('account restrictions', () => {
	it('builds the request body from the form', () => {
		expect(toRequest(true, ['b'], null, true)).toEqual({
			libraries: null,
			max_age_rating: null,
			block_unrated: false
		});
		expect(toRequest(false, ['b', 'a'], 7, true)).toEqual({
			libraries: ['a', 'b'],
			max_age_rating: 7,
			block_unrated: true
		});
		expect(toRequest(false, [], null, false).libraries).toEqual([]);
	});

	it('knows when an account is limited', () => {
		expect(isRestricted({ libraries: null, max_age_rating: null })).toBe(false);
		expect(isRestricted({ libraries: [], max_age_rating: null })).toBe(true);
		expect(isRestricted({ max_age_rating: 0 })).toBe(true);
		expect(isRestricted({ block_unrated: true })).toBe(false);
	});
});
