// Faza 18 — Fix match: a picked result must stay picked. The modal used to
// search from an `$effect` that also read the `searching` flag, so every
// finished search triggered the next one, and each new search cleared the
// pick — the administrator had to click "Use this match" faster than a
// round trip to the server.
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/svelte';
import FixMatchModal from './FixMatchModal.svelte';
import type { ProviderCandidate } from '$lib/media/identify';

const { searchProviders, identifyItem, fetchProviders } = vi.hoisted(() => ({
	searchProviders: vi.fn(),
	identifyItem: vi.fn(),
	fetchProviders: vi.fn()
}));

vi.mock('$lib/auth', () => ({ getSessionCookieClientSide: () => 'token' }));

vi.mock('$lib/media/identify', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/media/identify')>();
	return { ...actual, searchProviders, identifyItem };
});

vi.mock('$lib/admin/providers', async (importOriginal) => {
	const actual = await importOriginal<typeof import('$lib/admin/providers')>();
	return { ...actual, fetchProviders };
});

const candidate = (id: string, title: string, matchedBy = 'search'): ProviderCandidate => ({
	provider: 'imdb',
	provider_id: id,
	title,
	year: 2025,
	overview: '',
	poster_url: null,
	matched_by: matchedBy
});

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

function props(onIdentified = vi.fn()) {
	return {
		itemId: 'item-1',
		kind: 'series' as const,
		title: 'tt37543164',
		year: null,
		onClose: vi.fn(),
		onIdentified
	};
}

beforeEach(() => {
	searchProviders.mockReset();
	identifyItem.mockReset();
	fetchProviders.mockReset();
	fetchProviders.mockResolvedValue([]);
	searchProviders.mockResolvedValue({
		results: [candidate('tt37543164', 'Rojbers', 'id'), candidate('tt0000002', 'Rojbers 2')],
		failures: [],
		id_kind: 'imdb'
	});
});

describe('FixMatchModal', () => {
	it('searches once on open and keeps the pick until the admin confirms', async () => {
		const onIdentified = vi.fn();
		identifyItem.mockResolvedValue({
			item_id: 'item-1',
			title: 'Rojbers',
			year: 2025,
			items_updated: 8,
			image_saved: true,
			warnings: []
		});
		render(FixMatchModal, props(onIdentified));

		const second = await screen.findByRole('option', { name: /Rojbers 2/ });
		await fireEvent.click(second);
		expect(second.getAttribute('aria-selected')).toBe('true');

		// Time for any background search to come back.
		await wait(150);
		expect(searchProviders).toHaveBeenCalledTimes(1);
		const stillPicked = screen.getByRole('option', { name: /Rojbers 2/ });
		expect(stillPicked.getAttribute('aria-selected')).toBe('true');

		await fireEvent.click(
			screen.getByRole('button', { name: /Use this match|Użyj tego dopasowania/ })
		);
		await wait(0);
		expect(identifyItem).toHaveBeenCalledWith('token', 'item-1', 'imdb', 'tt0000002');
		expect(onIdentified).toHaveBeenCalled();
	});

	it('preselects the only id match', async () => {
		render(FixMatchModal, props());
		const idMatch = await screen.findByRole('option', { name: /Rojbers \(2025\)/ });
		await wait(0);
		expect(idMatch.getAttribute('aria-selected')).toBe('true');
	});

	it('typing does not search or clear the pick; a new search keeps a pick that is still listed', async () => {
		render(FixMatchModal, props());
		const second = await screen.findByRole('option', { name: /Rojbers 2/ });
		await fireEvent.click(second);

		const input = screen.getByRole('textbox', { name: /Title or ID|Tytuł lub ID/ });
		await fireEvent.input(input, { target: { value: 'Rojbers' } });
		await wait(100);
		expect(searchProviders).toHaveBeenCalledTimes(1);
		expect(screen.getByRole('option', { name: /Rojbers 2/ }).getAttribute('aria-selected')).toBe(
			'true'
		);

		await fireEvent.submit(input.closest('form')!);
		await wait(50);
		expect(searchProviders).toHaveBeenCalledTimes(2);
		expect(screen.getByRole('option', { name: /Rojbers 2/ }).getAttribute('aria-selected')).toBe(
			'true'
		);
	});
});
