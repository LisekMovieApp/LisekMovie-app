<!--
  Inline SVG icons on a 24×24 grid, stroked or filled with `currentColor`,
  so they follow text colour, hover and focus states. Decorative: the
  button around an icon carries the accessible name.
-->
<script lang="ts" module>
	export type IconName =
		| 'play'
		| 'pause'
		| 'rewind'
		| 'forward'
		| 'volume'
		| 'volume_low'
		| 'muted'
		| 'fullscreen'
		| 'fullscreen_exit'
		| 'subtitles'
		| 'tune'
		| 'pip'
		| 'back'
		| 'chevron_left'
		| 'chevron_right'
		| 'check'
		| 'info'
		| 'close'
		| 'menu'
		| 'plus';

	interface Shape {
		d: string;
		fill?: boolean;
		label?: string;
	}

	const SHAPES: Record<IconName, Shape> = {
		play: {
			d: 'M7 4.5v15a1 1 0 0 0 1.5.86l12.5-7.5a1 1 0 0 0 0-1.72L8.5 3.64A1 1 0 0 0 7 4.5z',
			fill: true
		},
		pause: { d: 'M6 4h4v16H6zM14 4h4v16h-4z', fill: true },
		rewind: { d: 'M4 12a8 8 0 1 0 2.4-5.7M4 4v4.5h4.5', label: '10' },
		forward: { d: 'M20 12a8 8 0 1 1-2.4-5.7M20 4v4.5h-4.5', label: '10' },
		volume: { d: 'M3 9.5v5h4l5 4.5v-14L7 9.5H3zM16 8.5a5 5 0 0 1 0 7M18.8 5.7a9 9 0 0 1 0 12.6' },
		volume_low: { d: 'M3 9.5v5h4l5 4.5v-14L7 9.5H3zM16 8.5a5 5 0 0 1 0 7' },
		muted: { d: 'M3 9.5v5h4l5 4.5v-14L7 9.5H3zM16 9.5l5 5M21 9.5l-5 5' },
		fullscreen: { d: 'M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5' },
		fullscreen_exit: { d: 'M9 4v5H4M15 4v5h5M15 20v-5h5M9 20v-5H4' },
		subtitles: {
			d: 'M3.5 5.5h17a1 1 0 0 1 1 1v11a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1v-11a1 1 0 0 1 1-1zM6.5 14.5h4M13 14.5h4.5M6.5 11h7M15.5 11h2'
		},
		tune: { d: 'M4 7h9M17 7h3M15 5v4M4 17h3M11 17h9M9 15v4' },
		pip: {
			d: 'M3.5 5h17a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1h-17a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zM12.5 11.5h6v5h-6z'
		},
		back: { d: 'M19 12H5M11 18l-6-6 6-6' },
		chevron_left: { d: 'M15 5l-7 7 7 7' },
		chevron_right: { d: 'M9 5l7 7-7 7' },
		check: { d: 'M5 12.5l4.5 4.5L19 7.5' },
		info: { d: 'M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18zM12 11v5.5M12 7.5v.5' },
		close: { d: 'M6 6l12 12M18 6L6 18' },
		menu: { d: 'M4 7h16M4 12h16M4 17h16' },
		plus: { d: 'M12 5v14M5 12h14' }
	};
</script>

<script lang="ts">
	interface Props {
		name: IconName;
		class?: string;
	}

	let { name, class: className = 'size-6' }: Props = $props();
	const shape = $derived(SHAPES[name]);
</script>

<svg
	viewBox="0 0 24 24"
	class={className}
	aria-hidden="true"
	fill={shape.fill ? 'currentColor' : 'none'}
	stroke={shape.fill ? 'none' : 'currentColor'}
	stroke-width="2"
	stroke-linecap="round"
	stroke-linejoin="round"
>
	<path d={shape.d} />
	{#if shape.label}
		<text
			x="12"
			y="15.2"
			text-anchor="middle"
			font-size="7"
			font-weight="700"
			fill="currentColor"
			stroke="none">{shape.label}</text
		>
	{/if}
</svg>
