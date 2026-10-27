import { describe, expect, it, vi } from 'vitest';
import { dismissToast, showToast, toasts } from './toast.svelte';

describe('toasts', () => {
	it('shows, times out and dismisses', () => {
		vi.useFakeTimers();
		const ok = showToast('success', 'Saved');
		const failed = showToast('error', 'Failed');
		expect(toasts.map((t) => t.message)).toEqual(['Saved', 'Failed']);
		vi.advanceTimersByTime(7000);
		expect(toasts.map((t) => t.id)).toEqual([failed]);
		dismissToast(failed);
		expect(toasts).toHaveLength(0);
		for (let i = 0; i < 7; i++) showToast('warning', `w${i}`, 0);
		expect(toasts).toHaveLength(5);
		expect(toasts[0].message).toBe('w2');
		toasts.splice(0);
		expect(ok).toBeLessThan(failed);
		vi.useRealTimers();
	});
});
