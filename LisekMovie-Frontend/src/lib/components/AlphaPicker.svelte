<!--
  Alpha picker (Faza 12): `#` and A–Z, jumping a library grid to the first
  title of a letter. Letters without titles are disabled, so the remote's
  up/down skip them. Three shapes, one per screen size (the grid page
  shows the one that fits):

  - `vertical`: a slim column for desktop and TV, sticky at the grid's
    right edge, like Jellyfin's. A `navGroup` contained vertically: up and
    down stay in the column, left goes back to the posters, and coming
    back lands on the last letter (or the active one). Right is a wall.
  - `horizontal`: a scrollable strip for tablets, pinned under the top bar.
  - `menu`: a "Jump to…" pill for phones. A native select sits on top of
    it, invisible, so the phone's own picker opens and no screen height is
    spent on 27 buttons.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { navGroup } from '$lib/spatial/spatial';
	import { directionOf } from '$lib/spatial/geometry';
	import { ALPHA_LETTERS } from '$lib/media/alpha';

	interface Props {
		/** Letters that have titles; the rest are disabled. */
		available: ReadonlySet<string>;
		/** The letter the viewer is at (highlighted), if known. */
		active: string | null;
		orientation: 'vertical' | 'horizontal' | 'menu';
		onpick: (letter: string) => void;
	}

	let { available, active, orientation, onpick }: Props = $props();

	let strip = $state<HTMLElement>();

	function label(letter: string): string {
		return letter === '#' ? t('alpha.other') : t('alpha.letter', { letter });
	}

	// Keep the active letter visible in the tablet strip.
	$effect(() => {
		if (orientation !== 'horizontal' || !strip || !active) return;
		const button = strip.querySelector<HTMLElement>(`[data-letter="${CSS.escape(active)}"]`);
		button?.scrollIntoView?.({ block: 'nearest', inline: 'center', behavior: 'smooth' });
	});

	const letterBase =
		'grid shrink-0 place-items-center rounded-full font-semibold leading-none transition-colors focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-gold disabled:cursor-default';
	function tone(letter: string): string {
		if (!available.has(letter)) return 'text-neutral-700';
		return active === letter
			? 'bg-gold text-black'
			: 'text-neutral-300 hover:bg-white/10 hover:text-gold focus-visible:bg-white/15 focus-visible:text-gold';
	}
</script>

{#if orientation === 'menu'}
	<label
		class="relative flex items-center gap-2 rounded-full bg-surface/95 py-2.5 pr-4 pl-4 text-sm text-neutral-100 shadow-[0_8px_24px_rgb(0_0_0/0.6)] ring-1 ring-white/15 backdrop-blur focus-within:ring-2 focus-within:ring-gold"
	>
		<span>{t('alpha.jump')}</span>
		{#if active}
			<span class="grid size-6 place-items-center rounded-full bg-gold text-xs font-bold text-black"
				>{active}</span
			>
		{/if}
		<select
			class="absolute inset-0 cursor-pointer opacity-0"
			aria-label={t('alpha.label')}
			value={active ?? ''}
			onchange={(event) => {
				const value = (event.currentTarget as HTMLSelectElement).value;
				if (value) onpick(value);
			}}
		>
			<option value="" disabled>{t('alpha.jump')}</option>
			{#each ALPHA_LETTERS as letter (letter)}
				<option value={letter} disabled={!available.has(letter)}
					>{letter === '#' ? t('alpha.other_short') : letter}</option
				>
			{/each}
		</select>
	</label>
{:else}
	<nav
		bind:this={strip}
		aria-label={t('alpha.label')}
		use:navGroup={{ contain: orientation === 'vertical' ? 'vertical' : 'horizontal' }}
		class={orientation === 'vertical'
			? 'flex h-full w-8 flex-col items-center rounded-full bg-white/[0.04] py-1.5 ring-1 ring-white/10'
			: 'flex [scrollbar-width:none] gap-1 overflow-x-auto [&::-webkit-scrollbar]:hidden'}
	>
		{#each ALPHA_LETTERS as letter (letter)}
			<button
				type="button"
				data-letter={letter}
				data-nav-default={active === letter ? '' : undefined}
				disabled={!available.has(letter)}
				aria-current={active === letter ? 'true' : undefined}
				aria-label={label(letter)}
				title={label(letter)}
				onclick={() => onpick(letter)}
				onkeydown={(event) => {
					// The column sits at the screen's right edge: nothing lies
					// to its right, so "right" is a wall rather than a jump to
					// some far-off row higher up the page.
					if (orientation === 'vertical' && directionOf(event) === 'right') event.preventDefault();
				}}
				class="{letterBase} {tone(letter)} {orientation === 'vertical'
					? 'max-h-7 min-h-0 w-7 flex-1 text-[0.7rem]'
					: 'size-8 text-xs'}"
			>
				{letter}
			</button>
		{/each}
	</nav>
{/if}
