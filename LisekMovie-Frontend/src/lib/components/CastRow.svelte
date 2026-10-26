<!--
  "Obsada i ekipa" on a detail page (Faza 8): directors, writers and the
  cast as a horizontal carousel of PersonCards. Reuses `Carousel`, so the
  row is a spatial-navigation group (left/right stay in the row, coming
  back lands on the card left last) and a focused card slides into view.
  Renders nothing without credits.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import type { Credit } from '$lib/media/people';
	import Carousel from './Carousel.svelte';
	import PersonCard from './PersonCard.svelte';

	interface Props {
		credits: Credit[];
		token: string;
	}

	let { credits, token }: Props = $props();
</script>

{#if credits.length > 0}
	<section class="group/row relative mt-2" aria-labelledby="cast-title">
		<h2 id="cast-title" class="mb-2 px-4 text-lg font-semibold text-neutral-100 md:px-8">
			{t('people.cast_title')}
		</h2>
		<Carousel>
			{#each credits as credit (`${credit.person_id}:${credit.role}`)}
				<PersonCard {credit} {token} />
			{/each}
		</Carousel>
	</section>
{/if}
