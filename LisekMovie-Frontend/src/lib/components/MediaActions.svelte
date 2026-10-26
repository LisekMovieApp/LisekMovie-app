<!--
  Watched / favourite toggles of a detail page, as round icon buttons
  (Faza 9, the reference layout's ✓ and ♥). Reports the saved state up
  (`onstate`), so the page can offer "Start over" for a started title.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { api, type MediaState } from '$lib/playback/api';
	import Icon from './Icon.svelte';
	import PlaystateButton from './PlaystateButton.svelte';
	import { TOGGLE_SHAPE, toggleTone } from './toggleTone';

	interface Props {
		itemId: string;
		token: string;
		onstate?: (state: MediaState | null) => void;
	}

	let { itemId, token, onstate }: Props = $props();
	let savedState = $state<MediaState | null>(null);
	let busy = $state(false);
	let failed = $state(false);

	$effect(() => {
		const id = itemId;
		const sessionToken = token;
		let active = true;
		savedState = null;
		failed = false;
		busy = false;
		onstate?.(null);
		api<MediaState>(sessionToken, `/api/media/${encodeURIComponent(id)}/state`)
			.then((result) => {
				if (!active) return;
				savedState = result;
				onstate?.(result);
			})
			.catch(() => {
				if (active) failed = true;
			});
		return () => {
			active = false;
		};
	});

	async function toggle(key: 'watched' | 'favorite') {
		if (!savedState || busy) return;
		const id = itemId;
		const value = !savedState[key];
		busy = true;
		failed = false;
		try {
			await api<void>(token, `/api/media/${encodeURIComponent(id)}/state`, 'PUT', {
				[key]: value
			});
			if (itemId === id && savedState) {
				savedState = { ...savedState, [key]: value };
				onstate?.(savedState);
			}
		} catch {
			if (itemId === id) failed = true;
		} finally {
			if (itemId === id) busy = false;
		}
	}
</script>

{#if savedState}
	<PlaystateButton played={savedState.watched} disabled={busy} onclick={() => toggle('watched')} />
	<button
		type="button"
		class="{TOGGLE_SHAPE} size-11 {toggleTone('favorite', savedState.favorite)}"
		disabled={busy}
		aria-pressed={savedState.favorite}
		aria-label={t(savedState.favorite ? 'player.unfavorite' : 'player.favorite')}
		title={t(savedState.favorite ? 'player.unfavorite' : 'player.favorite')}
		onclick={() => toggle('favorite')}
	>
		<Icon name={savedState.favorite ? 'heart_filled' : 'heart'} class="size-6" />
	</button>
{/if}
{#if failed}<p role="alert" class="text-sm text-gold">{t('player.save_failed')}</p>{/if}
