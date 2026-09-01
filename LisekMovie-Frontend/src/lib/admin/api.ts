// Shared bits between every `$lib/admin/*` API-client module (`providers.ts`,
// `libraries.ts`, …): the one error type callers can `instanceof`-check and
// the "throw it on a non-2xx response" helper both were built once inline in
// `providers.ts` and then duplicated verbatim into `libraries.ts` — pulled
// out here instead so there's a single definition (and a single place to
// extend it, e.g. with a parsed `error_code` from the body, if a caller ever
// needs one).

export class AdminApiError extends Error {
	constructor(public status: number) {
		super(`Admin API request failed with status ${status}`);
	}
}

export async function assertOk(response: Response): Promise<void> {
	if (!response.ok) {
		throw new AdminApiError(response.status);
	}
}
