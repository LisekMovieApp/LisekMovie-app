<!--
  Temporary Faza 1 scaffold-verification page: proves the brand palette
  (Tailwind @theme tokens) and logo asset pipeline work end to end.
  Replaced entirely by the real Home screen in Faza 4 (see roadmap).
-->
<script lang="ts">
	import logoText from '$lib/assets/logo_text.svg';
</script>

<main class="flex min-h-screen flex-col items-center justify-center gap-6 bg-base px-6">
	<img src={logoText} alt="LisekMovie" class="w-64" />
	<p class="text-sm tracking-wide text-gold uppercase">Faza 1 — toolchain scaffold</p>
</main>
