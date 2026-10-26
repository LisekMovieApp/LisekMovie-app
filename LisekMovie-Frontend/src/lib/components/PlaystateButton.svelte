<!--
  The watched (✓) toggle (Faza 12), shared by the detail page, season
  cards, the season page and episode rows. Watched shows as a solid gold
  disc with a black check (see `toggleTone.ts` for why the old recoloured
  icon stayed white). `aria-pressed` carries the state for screen readers.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import Icon from './Icon.svelte';
	import { TOGGLE_SHAPE, toggleTone, type ToggleSurface } from './toggleTone';

	interface Props {
		played: boolean;
		/** `sm` 36 px, `md` 40 px, `lg` 44 px. */
		size?: 'sm' | 'md' | 'lg';
		surface?: ToggleSurface;
		disabled?: boolean;
		/** Whether the visible tooltip is shown (dense lists skip it). */
		tooltip?: boolean;
		onclick: () => void;
	}

	let {
		played,
		size = 'lg',
		surface = 'plain',
		disabled = false,
		tooltip = true,
		onclick
	}: Props = $props();

	const sizes = { sm: 'size-9', md: 'size-10', lg: 'size-11' };
	const label = $derived(t(played ? 'player.unwatched' : 'player.watched'));
</script>

<button
	type="button"
	class="{TOGGLE_SHAPE} {sizes[size]} {toggleTone('watched', played, surface)}"
	{disabled}
	aria-pressed={played}
	aria-label={label}
	title={tooltip ? label : undefined}
	{onclick}
>
	<Icon name="check" class={size === 'lg' ? 'size-6' : 'size-5'} />
</button>
