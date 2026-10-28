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
		| 'plus'
		| 'next'
		| 'stack'
		| 'lock'
		| 'download'
		| 'external'
		| 'heart'
		| 'heart_filled'
		| 'more'
		| 'star'
		| 'replay'
		| 'search'
		| 'refresh'
		| 'user'
		| 'settings'
		| 'bell'
		| 'bell_off'
		| 'shield'
		| 'logout'
		| 'camera'
		| 'trash'
		| 'globe'
		| 'key';

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
		plus: { d: 'M12 5v14M5 12h14' },
		next: { d: 'M5 4.5v15a.7.7 0 0 0 1.1.6L16 12.6a.7.7 0 0 0 0-1.2L6.1 3.9A.7.7 0 0 0 5 4.5zM17 4h2.5v16H17z', fill: true },
		stack: { d: 'M4 8.5h16v11H4zM6.5 6h11M9 3.5h6' },
		lock: { d: 'M6.5 10.5h11a1 1 0 0 1 1 1v8a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1zM8.5 10.5V7.5a3.5 3.5 0 0 1 7 0v3M12 14.5v2.5' },
		download: { d: 'M12 4v11M7.5 10.5L12 15l4.5-4.5M5 19.5h14' },
		heart: { d: 'M12 20.5s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7.6a4.3 4.3 0 0 1 7.5 2.7c0 5.6-7.5 10.2-7.5 10.2z' },
		heart_filled: { d: 'M12 20.5s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7.6a4.3 4.3 0 0 1 7.5 2.7c0 5.6-7.5 10.2-7.5 10.2z', fill: true },
		more: {
			d: 'M12 4.5a1.75 1.75 0 1 0 0 3.5 1.75 1.75 0 0 0 0-3.5zM12 10.25a1.75 1.75 0 1 0 0 3.5 1.75 1.75 0 0 0 0-3.5zM12 16a1.75 1.75 0 1 0 0 3.5 1.75 1.75 0 0 0 0-3.5z',
			fill: true
		},
		star: {
			d: 'M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8-4.3-4.1 5.9-.9z',
			fill: true
		},
		search: { d: 'M10.5 17.5a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM15.5 15.5L20.5 20.5' },
		replay: { d: 'M4 12a8 8 0 1 0 2.4-5.7M4 4v4.5h4.5' },
		external: { d: 'M14 4.5h5.5V10M19.5 4.5L11 13M17 14v4.5a1 1 0 0 1-1 1H5.5a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1H10' },
		refresh: { d: 'M20 11.5A8 8 0 0 0 6.3 6.3L4 8.5M4 4v4.5h4.5M4 12.5a8 8 0 0 0 13.7 5.2L20 15.5M20 20v-4.5h-4.5' },
		user: { d: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4.5 20a7.5 7.5 0 0 1 15 0' },
		settings: {
			d: 'M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z'
		},
		bell: { d: 'M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15zM10 20.5a2 2 0 0 0 4 0' },
		bell_off: { d: 'M8.6 5.6A6 6 0 0 1 18 11v4M6 11v5.5l-1.5 2H17M10 20.5a2 2 0 0 0 4 0M3.5 3.5l17 17' },
		shield: { d: 'M12 3.5l7 2.8v5.2c0 4.4-3 7.8-7 9-4-1.2-7-4.6-7-9V6.3zM9 12l2 2 4-4' },
		logout: { d: 'M14 4.5H6.5a1 1 0 0 0-1 1v13a1 1 0 0 0 1 1H14M10 12h10M16.5 8.5L20 12l-3.5 3.5' },
		camera: { d: 'M4.5 8a1 1 0 0 1 1-1h2.7l1.5-2h4.6l1.5 2h2.7a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1h-13a1 1 0 0 1-1-1zM12 16.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7z' },
		trash: { d: 'M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1L17.5 7M10 11v6M14 11v6' },
		globe: { d: 'M12 20.5a8.5 8.5 0 1 0 0-17 8.5 8.5 0 0 0 0 17zM3.5 12h17M12 3.5c2.3 2.4 3.5 5.2 3.5 8.5s-1.2 6.1-3.5 8.5c-2.3-2.4-3.5-5.2-3.5-8.5S9.7 5.9 12 3.5z' },
		key: { d: 'M8.5 15.5a4 4 0 1 1 3.6-5.8L20 9.7V13h-2.5v2.5H15V13h-2.9a4 4 0 0 1-3.6 2.5z' }
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
