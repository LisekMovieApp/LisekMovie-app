<!--
  The "⋮" button of a detail page (Faza 9): a small menu for the less
  frequent actions (download, edit metadata, fix match). While open it is
  its own spatial-navigation scope (the remote stays inside it, the first
  entry takes focus); Escape, Back or a click outside closes it and gives
  focus back to the button.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { navScope } from '$lib/spatial/spatial';
	import Icon from './Icon.svelte';

	interface Entry {
		labelKey: string;
		onselect: () => void;
		disabled?: boolean;
	}

	interface Props {
		entries: Entry[];
		/** Smaller round button on a dark chip (cards, list rows). */
		compact?: boolean;
	}

	let { entries, compact = false }: Props = $props();
	let open = $state(false);
	let trigger: HTMLButtonElement | undefined = $state();
	let panel: HTMLDivElement | undefined = $state();

	function close(refocus = true) {
		open = false;
		if (refocus) trigger?.focus({ preventScroll: true });
	}

	function choose(entry: Entry) {
		close();
		entry.onselect();
	}

	$effect(() => {
		if (!open) return;
		const onPointer = (event: PointerEvent) => {
			const target = event.target as Node;
			if (!panel?.contains(target) && !trigger?.contains(target)) close(false);
		};
		const onKey = (event: KeyboardEvent) => {
			if (event.key === 'Escape') {
				event.preventDefault();
				close();
			}
		};
		const onBack = (event: Event) => {
			event.preventDefault();
			close();
		};
		document.addEventListener('pointerdown', onPointer, true);
		window.addEventListener('keydown', onKey);
		window.addEventListener('spatialback', onBack);
		return () => {
			document.removeEventListener('pointerdown', onPointer, true);
			window.removeEventListener('keydown', onKey);
			window.removeEventListener('spatialback', onBack);
		};
	});
</script>

{#if entries.length > 0}
	<div class="relative">
		<button
			bind:this={trigger}
			type="button"
			aria-haspopup="menu"
			aria-expanded={open}
			aria-label={t('details.more')}
			title={t('details.more')}
			onclick={() => (open = !open)}
			class="grid place-items-center rounded-full text-neutral-100 transition-colors hover:text-gold focus-visible:text-gold focus-visible:outline-2 focus-visible:outline-gold {compact
				? 'size-9 bg-black/60 backdrop-blur hover:bg-black/80 focus-visible:bg-black/90'
				: 'size-11 hover:bg-white/10 focus-visible:bg-white/15'}"
		>
			<Icon name="more" class={compact ? 'size-5' : 'size-6'} />
		</button>
		{#if open}
			<div
				bind:this={panel}
				role="menu"
				use:navScope={{ autofocus: true }}
				class="absolute top-full right-0 z-30 mt-2 min-w-56 overflow-hidden rounded-md border border-white/10 bg-neutral-900/95 py-1 shadow-[0_15px_40px_rgb(0_0_0/0.6)] backdrop-blur"
			>
				{#each entries as entry (entry.labelKey)}
					<button
						type="button"
						role="menuitem"
						disabled={entry.disabled}
						onclick={() => choose(entry)}
						class="block w-full px-4 py-2.5 text-left text-sm text-neutral-100 hover:bg-white/10 hover:text-gold focus-visible:bg-white/10 focus-visible:text-gold focus-visible:outline-none disabled:opacity-50"
					>
						{t(entry.labelKey)}
					</button>
				{/each}
			</div>
		{/if}
	</div>
{/if}
