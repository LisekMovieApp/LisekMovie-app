import { describe, expect, it } from 'vitest';
import {
	displayMessage,
	extraFields,
	formatBytes,
	logQuery,
	matchesText,
	type LogEntry
} from './logs';

const entry: LogEntry = {
	timestamp: '2026-10-27T12:00:00Z',
	level: 'ERROR',
	source: 'ffprobe',
	target: 'ffprobe',
	message: '[FFPROBE] Invalid data',
	fields: { context: '/media/Film.mkv', empty: '', missing: null, count: 3 }
};

describe('log viewer helpers', () => {
	it('sends filters only when they narrow', () => {
		expect(
			logQuery(
				['backend', 'frontend', 'ffmpeg', 'ffprobe'],
				['error', 'warn', 'info', 'debug'],
				100
			)
		).toBe('limit=100');
		expect(logQuery(['ffmpeg', 'ffprobe'], ['error'], 50)).toBe(
			'limit=50&source=ffmpeg%2Cffprobe&level=error'
		);
	});

	it('shows only the fields with a value and searches them', () => {
		expect(extraFields(entry)).toEqual([
			['context', '/media/Film.mkv'],
			['count', '3']
		]);
		expect(matchesText(entry, 'invalid')).toBe(true);
		expect(matchesText(entry, 'film.mkv')).toBe(true);
		expect(matchesText(entry, 'count=3')).toBe(true);
		expect(matchesText(entry, 'nothing')).toBe(false);
		expect(matchesText(entry, '  ')).toBe(true);
	});

	it('does not repeat the source tag', () => {
		expect(displayMessage(entry)).toBe('Invalid data');
		expect(displayMessage({ ...entry, source: 'backend' })).toBe('[FFPROBE] Invalid data');
	});

	it('formats file sizes', () => {
		expect(formatBytes(512)).toBe('512 B');
		expect(formatBytes(2048)).toBe('2.0 KB');
		expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
	});
});
