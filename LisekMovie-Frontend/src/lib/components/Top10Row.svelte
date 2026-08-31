<!--
  The "Top 10" row — same horizontally-scrolling carousel mechanics as
  MediaRow, but each poster carries a giant rank number instead of a plain
  title, matching the reference layout. The rank numbers are real, final UI
  (a position 1-10 is structural, not unknown data waiting on the library
  scanner) — only the poster art underneath is a skeleton, same reasoning
  as the rest of Faza 4. Posters here are portrait, not the landscape
  aspect-video of SkeletonCard/MediaRow: that's deliberate, matching the
  reference's Top-10-specific visual language rather than reusing the
  generic thumbnail shape.

  Same `snap-proximity` note as MediaRow: mandatory snap fights a JS
  smooth scrollBy in Chromium and turns the animation into a jump.
-->
<script lang="ts">
	import { t } from '$lib/i18n';

	const ranks = Array.from({ length: 10 }, (_, i) => i + 1);

	let scrollEl: HTMLDivElement | undefined = $state();

	function scrollByPage(direction: 1 | -1) {
		if (!scrollEl) return;
		scrollEl.scrollBy({ left: direction * scrollEl.clientWidth * 0.9, behavior: 'smooth' });
	}
</script>

<section class="group/row relative">
	<h2 class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">{t('home.row_top10')}</h2>

	<div class="relative">
		<div
			bind:this={scrollEl}
			class="no-scrollbar flex snap-x snap-proximity items-end overflow-x-auto scroll-smooth px-4 md:px-8"
		>
			{#each ranks as rank (rank)}
				<div class="flex shrink-0 snap-start items-end">
					<!--
					  Transparent fill + a gold stroke, rather than a plain
					  translucent grey number: the outline reads clearly against any
					  poster art behind it without competing for attention with the
					  poster itself once real thumbnails land here.
					-->
					<span
						aria-hidden="true"
						class="-mr-5 text-[6.5rem] leading-none font-black text-transparent select-none [-webkit-text-stroke:3px_var(--color-gold)] sm:-mr-7 sm:text-[8.5rem] md:-mr-9 md:text-[10rem]"
					>
						{rank}
					</span>
					<div
						class="aspect-[2/3] w-[110px] shrink-0 animate-pulse rounded-md bg-surface sm:w-[140px] md:w-[160px]"
					></div>
				</div>
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
