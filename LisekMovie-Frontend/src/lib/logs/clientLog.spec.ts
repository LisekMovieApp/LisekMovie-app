import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
	CLIENT_LOG_ENDPOINT,
	describeError,
	installErrorReporting,
	reportClientError,
	resetClientLog,
	shouldSend,
	type ClientReport
} from './clientLog';

const report = (message: string): ClientReport => ({ level: 'error', kind: 'error', message });

describe('browser error reports', () => {
	beforeEach(() => resetClientLog());
	afterEach(() => vi.unstubAllGlobals());

	it('sends one report per message every 30 s and ten a minute', () => {
		expect(shouldSend(report('a'), 1_000)).toBe(true);
		expect(shouldSend(report('a'), 20_000)).toBe(false);
		expect(shouldSend(report('a'), 31_001)).toBe(true);
		for (let i = 0; i < 8; i++) expect(shouldSend(report(`m${i}`), 40_000)).toBe(true);
		expect(shouldSend(report('one too many'), 50_000)).toBe(false);
		expect(shouldSend(report('next minute'), 62_000)).toBe(true);
	});

	it('describes anything thrown', () => {
		expect(describeError(new TypeError('x is undefined')).message).toBe(
			'TypeError: x is undefined'
		);
		expect(describeError('plain').message).toBe('plain');
		expect(describeError({ code: 7 }).message).toBe('{"code":7}');
	});

	it('posts to the server and never throws', async () => {
		const fetchMock = vi.fn().mockResolvedValue({ ok: true });
		reportClientError({ ...report('Boom'), stack: 'at x', url: 'http://h/p' }, fetchMock);
		const [url, options] = fetchMock.mock.calls[0];
		expect(url).toBe(CLIENT_LOG_ENDPOINT);
		expect(options.method).toBe('POST');
		expect(options.keepalive).toBe(true);
		expect(JSON.parse(options.body)).toMatchObject({
			message: 'Boom',
			stack: 'at x',
			url: 'http://h/p'
		});
		const failing = vi.fn().mockRejectedValue(new Error('offline'));
		expect(() => reportClientError(report('Other'), failing)).not.toThrow();
		reportClientError(report('   '), fetchMock);
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});

	it('listens to uncaught errors and rejections, but not its own', () => {
		const fetchMock = vi.fn().mockResolvedValue({ ok: true });
		vi.stubGlobal('fetch', fetchMock);
		const target = new EventTarget() as unknown as Window;
		const stop = installErrorReporting(target);
		const error = Object.assign(new Event('error'), {
			message: 'Uncaught ReferenceError: foo',
			error: new ReferenceError('foo'),
			filename: 'app.js',
			lineno: 3,
			colno: 9
		});
		target.dispatchEvent(error);
		expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toMatchObject({
			kind: 'error',
			source: 'app.js:3:9'
		});
		target.dispatchEvent(
			Object.assign(new Event('unhandledrejection'), { reason: new Error('nope') })
		);
		expect(JSON.parse(fetchMock.mock.calls[1][1].body).kind).toBe('unhandledrejection');
		target.dispatchEvent(
			Object.assign(new Event('unhandledrejection'), {
				reason: new Error(`fetch ${CLIENT_LOG_ENDPOINT} failed`)
			})
		);
		expect(fetchMock).toHaveBeenCalledTimes(2);
		stop();
		target.dispatchEvent(Object.assign(new Event('unhandledrejection'), { reason: 'later' }));
		expect(fetchMock).toHaveBeenCalledTimes(2);
	});
});
