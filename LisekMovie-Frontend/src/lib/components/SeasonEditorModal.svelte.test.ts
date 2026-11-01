// Faza 21 — the season editor: a changed name is locked on save, the
// poster lock can be switched, a year out of range is never sent.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import SeasonEditorModal from './SeasonEditorModal.svelte';
import type { SeasonDetail } from '$lib/admin/mediaItems';

const { fetchSeason, updateSeason } = vi.hoisted(() => ({
	fetchSeason: vi.fn(),
	updateSeason: vi.fn()
}));

vi.mock('$lib/auth', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/auth')>();
	return { ...actual, getSessionCookieClientSide: () => 'tok' };
});
vi.mock('$lib/admin/mediaItems', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/admin/mediaItems')>();
	return { ...actual, fetchSeason, updateSeason };
});

const season: SeasonDetail = {
	series_id: 's1',
	series_title: 'Rojbers',
	season: 2,
	name: 'Season 2',
	overview: 'From the provider',
	air_year: 2024,
	locked_fields: ['poster'],
	episode_count: 8,
	version: 3
};

const wait = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
	fetchSeason.mockReset().mockResolvedValue(season);
	updateSeason.mockReset().mockResolvedValue({ ok: true, updated: 1, auto_locked: ['name'] });
});

describe('SeasonEditorModal', () => {
	it('saves a renamed season with the name locked', async () => {
		const onSaved = vi.fn();
		const onClose = vi.fn();
		render(SeasonEditorModal, { itemId: 'ep1', season: 2, onClose, onSaved });
		await wait();
		expect(fetchSeason).toHaveBeenCalledWith('tok', 'ep1', 2);
		expect(screen.getByText('Rojbers · season 2 · 8 episodes')).toBeTruthy();
		const name = screen.getByRole('textbox', { name: 'Season name' }) as HTMLInputElement;
		expect(name.value).toBe('Season 2');
		await fireEvent.input(name, { target: { value: 'Powrót' } });
		// The poster was locked before: unlocking it is kept.
		await fireEvent.click(screen.getByRole('checkbox', { name: /Poster/ }));
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await wait();
		expect(updateSeason).toHaveBeenCalledWith('tok', 'ep1', 2, {
			name: 'Powrót',
			overview: 'From the provider',
			air_year: 2024,
			locked_fields: ['name']
		});
		expect(onSaved).toHaveBeenCalled();
		expect(onClose).toHaveBeenCalled();
	});

	it('does not send a year outside 1800-2200', async () => {
		render(SeasonEditorModal, { itemId: 'ep1', season: 2, onClose: vi.fn(), onSaved: vi.fn() });
		await wait();
		await fireEvent.input(screen.getByRole('spinbutton', { name: 'Year' }), {
			target: { value: '99' }
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await wait();
		expect(updateSeason).not.toHaveBeenCalled();
	});
});
