// Client hooks (Faza 17): every error of the app in a browser ends up in
// the server log ([FRONTEND], Admin → Logs) — uncaught errors and rejected
// promises from `installErrorReporting`, errors while loading or rendering
// a page from `handleError`.

import type { HandleClientError, ClientInit } from '@sveltejs/kit';
import { describeError, installErrorReporting, reportClientError } from '$lib/logs/clientLog';

export const init: ClientInit = () => {
	installErrorReporting(window);
};

export const handleError: HandleClientError = ({ error, event, status, message }) => {
	// A missing page is not an application error.
	if (status !== 404) {
		const described = describeError(error);
		reportClientError({
			level: 'error',
			kind: 'handle_error',
			message: `${described.message} (${status} on ${event.route.id ?? event.url.pathname})`,
			stack: described.stack,
			url: event.url.href
		});
	}
	return { message };
};
