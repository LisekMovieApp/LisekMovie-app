<!--
  Home screen — Faza 4. Hero banner + media rows, all skeleton/placeholder
  content: real items arrive once library scanning and metadata exist
  (Faza 5+). The account controls that used to live directly on this page
  (Faza 3 scaffold) now live in the navbar, in the root layout, so they
  show up on every page rather than just this one.
-->
<script lang="ts">
	import HeroBanner from '$lib/components/HeroBanner.svelte';
	import MediaRow from '$lib/components/MediaRow.svelte';
	import Top10Row from '$lib/components/Top10Row.svelte';

	const rows = [
		{ titleKey: 'home.row_recently_added', count: 12 },
		{ titleKey: 'home.row_recommended', count: 10 },
		{ titleKey: 'home.row_popular', count: 14 }
	];
</script>

<main class="min-h-screen bg-base pb-16">
	<HeroBanner />

	<div class="mt-8 flex flex-col gap-8">
		<Top10Row />
		{#each rows as row (row.titleKey)}
			<MediaRow titleKey={row.titleKey} count={row.count} />
		{/each}
	</div>
</main>
