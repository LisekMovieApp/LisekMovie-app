<!--
  The big banner at the top of Home. Faza 4 only proves the layout — the
  background gradient, the bottom vignette that blends into the page, and
  the title/metadata/button placement — because there's no featured item to
  show yet (that needs library scanning + metadata, Faza 5+). Title and
  metadata are skeleton bars rather than invented text; Play/More Info are
  real buttons, disabled, so their exact size and position is already final
  once real data wires them up.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
</script>

<section class="relative h-[80vh] max-h-[760px] min-h-[420px] w-full overflow-hidden bg-surface">
	<!-- Background placeholder: a brand-tinted gradient stands in for the featured item's backdrop image. -->
	<div class="absolute inset-0 bg-gradient-to-br from-surface via-black to-surface"></div>
	<div
		class="absolute inset-0 bg-[radial-gradient(ellipse_at_30%_20%,rgba(189,162,77,0.14),transparent_60%)]"
	></div>

	<!-- Top scrim, keeps the transparent navbar's logo/links readable over any background. -->
	<div class="absolute inset-x-0 top-0 h-32 bg-gradient-to-b from-black/70 to-transparent"></div>

	<!-- Bottom vignette — the winieta that blends the banner into the page background. -->
	<div
		class="absolute inset-x-0 bottom-0 h-2/3 bg-gradient-to-t from-base via-base/70 to-transparent"
	></div>

	<div class="absolute bottom-0 left-0 flex w-full max-w-xl flex-col gap-4 p-4 md:p-8 lg:p-12">
		<div class="flex flex-col gap-2" aria-hidden="true">
			<div class="h-8 w-3/4 animate-pulse rounded bg-white/20 md:h-10"></div>
			<div class="h-8 w-1/2 animate-pulse rounded bg-white/20 md:h-10"></div>
		</div>
		<div class="h-4 w-48 animate-pulse rounded bg-white/10" aria-hidden="true"></div>

		<div class="mt-2 flex gap-3">
			<button
				type="button"
				disabled
				class="rounded-md bg-gold px-6 py-2.5 font-medium text-black transition-colors hover:bg-gold-dim disabled:cursor-not-allowed disabled:opacity-50"
			>
				{t('home.play')}
			</button>
			<button
				type="button"
				disabled
				class="rounded-md bg-white/20 px-6 py-2.5 font-medium text-white backdrop-blur-sm transition-colors hover:bg-white/30 disabled:cursor-not-allowed disabled:opacity-50"
			>
				{t('home.more_info')}
			</button>
		</div>
	</div>
</section>
