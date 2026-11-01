// Faza 20 — the metadata editor: an episode edits its own title (not the
// series it belongs to), a series edits every episode, changed fields are
// locked automatically, pictures can be replaced.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import MediaItemEditorModal from './MediaItemEditorModal.svelte';
import type { MediaItemDetail } from '$lib/admin/mediaItems';

const { fetchMediaItem, updateMediaItem } = vi.hoisted(() => ({
	fetchMediaItem: vi.fn(),
	updateMediaItem: vi.fn()
}));

vi.mock('$lib/auth', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/auth')>();
	return { ...actual, getSessionCookieClientSide: () => 'tok' };
});
vi.mock('$lib/admin/mediaItems', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/admin/mediaItems')>();
	return { ...actual, fetchMediaItem, updateMediaItem };
});

const episode: MediaItemDetail = {
	id: 'ep1',
	library_id: 'lib',
	item_type: 'episode',
	title: 'Rojbers',
	original_title: null,
	year: 2025,
	overview: 'Serial',
	season: 1,
	episode: 3,
	metadata_locked: false,
	forced_provider: null,
	locked_fields: [],
	series_id: 's1',
	series_title: 'Rojbers',
	episode_count: 8,
	episode_title: 'Pilot',
	episode_overview: 'Od dostawcy',
	episode_air_date: '2025-02-01',
	version: 5
};

const wait = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
	fetchMediaItem.mockReset().mockResolvedValue(episode);
	updateMediaItem.mockReset().mockResolvedValue({ ok: true, updated: 1, auto_locked: [] });
});

describe('MediaItemEditorModal', () => {
	it('edits an episode title without touching the series', async () => {
		const onSaved = vi.fn();
		render(MediaItemEditorModal, {
			itemId: 'ep1',
			providers: [],
			scope: 'episode',
			onClose: vi.fn(),
			onSaved
		});
		await wait();
		expect(screen.getByText('Edit episode metadata')).toBeTruthy();
		expect(screen.getByText('Rojbers · S01E03')).toBeTruthy();
		const title = screen.getByRole('textbox', { name: 'Episode title' }) as HTMLInputElement;
		expect(title.value).toBe('Pilot');
		await fireEvent.input(title, { target: { value: 'Najwyższy czas' } });
		// Changed: locked automatically, shown before saving.
		const active = screen.getByRole('checkbox', { name: /Episode title/ }) as HTMLInputElement;
		expect(active.checked).toBe(false);
		expect(screen.getByText(/Changed fields are locked/)).toBeTruthy();
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await wait();
		expect(updateMediaItem).toHaveBeenCalledWith('tok', 'ep1', {
			scope: 'episode',
			episode_title: 'Najwyższy czas',
			episode_overview: 'Od dostawcy',
			episode_air_date: '2025-02-01',
			locked_fields: ['episode_title']
		});
		expect(onSaved).toHaveBeenCalled();
	});

	it('edits the whole series from the series page', async () => {
		updateMediaItem.mockResolvedValue({ ok: true, updated: 8, auto_locked: ['title'] });
		render(MediaItemEditorModal, {
			itemId: 'ep1',
			providers: [],
			scope: 'series',
			onClose: vi.fn(),
			onSaved: vi.fn()
		});
		await wait();
		expect(screen.getByText('Edit series metadata')).toBeTruthy();
		expect(screen.getByText('Changes apply to all 8 episodes of the series.')).toBeTruthy();
		await fireEvent.input(screen.getByRole('textbox', { name: /^Series title/ }), {
			target: { value: 'Rojbersi' }
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await wait();
		const [, , payload] = updateMediaItem.mock.calls[0];
		expect(payload).toMatchObject({ scope: 'series', title: 'Rojbersi', locked_fields: ['title'] });
	});

	it('offers poster, backdrop and logo for a series and the still for an episode', async () => {
		const first = render(MediaItemEditorModal, {
			itemId: 'ep1',
			providers: [],
			scope: 'series',
			onClose: vi.fn(),
			onSaved: vi.fn()
		});
		await wait();
		await fireEvent.click(screen.getByRole('tab', { name: 'Artwork' }));
		expect(screen.getAllByRole('region').map((r) => r.getAttribute('aria-label'))).toEqual([
			'Poster',
			'Backdrop',
			'Logo (transparent PNG works best)'
		]);
		first.unmount();
		render(MediaItemEditorModal, {
			itemId: 'ep1',
			providers: [],
			scope: 'episode',
			onClose: vi.fn(),
			onSaved: vi.fn()
		});
		await wait();
		await fireEvent.click(screen.getByRole('tab', { name: 'Artwork' }));
		expect(screen.getAllByRole('region').map((r) => r.getAttribute('aria-label'))).toEqual([
			'Episode still'
		]);
	});

	it('shows why a series rename was refused', async () => {
		const { AdminApiError } = await import('$lib/admin/api');
		updateMediaItem.mockRejectedValue(new AdminApiError(409, 'series_title_taken'));
		render(MediaItemEditorModal, {
			itemId: 'ep1',
			providers: [],
			scope: 'series',
			onClose: vi.fn(),
			onSaved: vi.fn()
		});
		await wait();
		await fireEvent.input(screen.getByRole('textbox', { name: /^Series title/ }), {
			target: { value: 'Inny' }
		});
		await fireEvent.click(screen.getByRole('button', { name: 'Save' }));
		await wait();
		expect(screen.getByRole('alert').textContent).toContain('already has this title');
	});
});
