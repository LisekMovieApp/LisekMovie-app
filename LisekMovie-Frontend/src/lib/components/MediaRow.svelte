<!--
  A horizontally-scrolling row of media: a title plus a scroll-snapping strip
  of SkeletonCard placeholders. Real content wiring (fetching library items,
  linking each card to a detail page) is Faza 5+ — this component only needs
  to prove the carousel mechanics (snap, smooth scroll, hidden scrollbar,
  edge scroll buttons) work and look right.

  Snap strength is deliberately `snap-proximity`, not `snap-mandatory`:
  mandatory snap fights a JS `scrollBy({ behavior: 'smooth' })` in Chromium
  — the browser resolves the snap correction by cutting straight to the
  target instead of letting the smooth animation play, so the arrow buttons
  visibly jumped instead of gliding. Proximity still snaps a scroll that
  ends up near a card edge (mouse wheel, touch, keyboard), it just doesn't
  fight an in-flight smooth scroll to get there.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import SkeletonCard from './SkeletonCard.svelte';

	interface Props {
		/** i18n key for the row's heading, e.g. "home.row_recently_added". */
		titleKey: string;
		/** How many placeholder cards to render — enough to overflow on a typical screen. */
		count?: number;
	}

	let { titleKey, count = 10 }: Props = $props();

	let scrollEl: HTMLDivElement | undefined = $state();

	function scrollByPage(direction: 1 | -1) {
		if (!scrollEl) return;
		scrollEl.scrollBy({ left: direction * scrollEl.clientWidth * 0.9, behavior: 'smooth' });
	}
</script>

<section class="group/row relative">
	<h2 class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">{t(titleKey)}</h2>

	<div class="relative">
		<div
			bind:this={scrollEl}
			class="no-scrollbar flex snap-x snap-proximity gap-2 overflow-x-auto scroll-smooth px-4 md:px-8"
		>
			{#each { length: count } as _, i (i)}
				<SkeletonCard />
			{/each}
		</div>

		<button
			type="button"
			aria-label={t('home.scroll_left')}
			onclick={() => scrollByPage(-1)}
			class="absolute inset-y-0 left-0 hidden w-10 items-center justify-center bg-gradient-to-r from-base/80 to-transparent text-neutral-100 opacity-0 transition-opacity group-hover/row:opacity-100 hover:text-gold md:flex md:w-14"
		>
			<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-6">
				<path d="M15 6l-6 6 6 6" stroke-linecap="round" stroke-linejoin="round" />
			</svg>
		</button>

		<button
			type="button"
			aria-label={t('home.scroll_right')}
			onclick={() => scrollByPage(1)}
			class="absolute inset-y-0 right-0 hidden w-10 items-center justify-center bg-gradient-to-l from-base/80 to-transparent text-neutral-100 opacity-0 transition-opacity group-hover/row:opacity-100 hover:text-gold md:flex md:w-14"
		>
			<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="size-6">
				<path d="M9 6l6 6-6 6" stroke-linecap="round" stroke-linejoin="round" />
			</svg>
		</button>
	</div>
</section>
