<!--
  A single-series column chart (Faza 13 statistics): viewing per day, per
  hour, drop-off per tenth of a title. HTML bars rather than SVG, so they
  stretch with the card on any screen without distorting text.

  One gold hue (one series, no legend needed: the card's title names it),
  4 px rounded tops on a shared baseline, 2 px gaps. Hovering or focusing
  a bar shows its label and value; every bar is in a visually hidden
  table for screen readers. Axis labels are thinned to fit.
-->
<script lang="ts">
	interface Bar {
		label: string;
		value: number;
		/** The value as shown in the tooltip ("1h 38m", "12"). */
		display: string;
		/** Shorter axis label (default: `label`). */
		tick?: string;
	}

	interface Props {
		bars: Bar[];
		/** Names the chart for screen readers. */
		label: string;
		/** Header of the hidden table's value column. */
		valueLabel: string;
		/** At most this many axis labels. */
		maxTicks?: number;
		/** Optional click on a bar (e.g. filter by that day). */
		onselect?: (index: number) => void;
	}

	let { bars, label, valueLabel, maxTicks = 8, onselect }: Props = $props();

	let hovered = $state<number | null>(null);
	const max = $derived(Math.max(0, ...bars.map((b) => b.value)));
	const step = $derived(Math.max(1, Math.ceil(bars.length / maxTicks)));
</script>

<div class="relative">
	<div
		class="flex h-40 items-end gap-[2px] border-b border-white/15 md:h-48"
		role="group"
		aria-label={label}
		onmouseleave={() => (hovered = null)}
	>
		{#each bars as bar, i (i)}
			<button
				type="button"
				tabindex={onselect ? 0 : -1}
				data-nav-skip={onselect ? undefined : ''}
				class="group relative flex h-full min-w-0 flex-1 items-end focus-visible:outline-none {onselect
					? 'cursor-pointer'
					: 'cursor-default'}"
				aria-label="{bar.label}: {bar.display}"
				onmouseenter={() => (hovered = i)}
				onfocus={() => (hovered = i)}
				onblur={() => (hovered = null)}
				onclick={() => onselect?.(i)}
			>
				<span
					class="block w-full rounded-t-[4px] transition-colors {hovered === i
						? 'bg-[#d6bb66]'
						: 'bg-gold'} group-focus-visible:outline-2 group-focus-visible:outline-offset-2 group-focus-visible:outline-white"
					style:height={`${max > 0 ? Math.max(bar.value > 0 ? 2 : 0, (bar.value / max) * 100) : 0}%`}
				></span>
			</button>
		{/each}
	</div>
	{#if hovered !== null && bars[hovered]}
		<div
			class="pointer-events-none absolute -top-2 z-10 -translate-x-1/2 -translate-y-full rounded-md bg-neutral-900 px-2.5 py-1.5 text-xs whitespace-nowrap text-neutral-100 shadow-lg ring-1 ring-white/15"
			style:left={`${((hovered + 0.5) / bars.length) * 100}%`}
		>
			<span class="text-neutral-400">{bars[hovered].label}</span>
			<span class="ml-1.5 font-semibold tabular-nums">{bars[hovered].display}</span>
		</div>
	{/if}
	<div class="mt-1.5 flex gap-[2px] text-[0.65rem] text-neutral-500" aria-hidden="true">
		{#each bars as bar, i (i)}
			<span class="min-w-0 flex-1 overflow-visible text-center whitespace-nowrap"
				>{i % step === 0 ? (bar.tick ?? bar.label) : ''}</span
			>
		{/each}
	</div>
	<table class="sr-only">
		<caption>{label}</caption>
		<tbody>
			<tr><th scope="col"></th><th scope="col">{valueLabel}</th></tr>
			{#each bars as bar, i (i)}
				<tr><th scope="row">{bar.label}</th><td>{bar.display}</td></tr>
			{/each}
		</tbody>
	</table>
</div>
