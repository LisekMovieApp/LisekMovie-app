<!--
  One person in a cast row (Faza 8): portrait photo, name and the part
  they play (or their crew role) underneath, linking to `/person/[id]`.
  Without a photo the card shows the person's initials. Focus (keyboard,
  TV remote) lifts the photo with a gold ring, like MediaCard.
-->
<script lang="ts">
	import { resolve } from '$app/paths';
	import { t } from '$lib/i18n';
	import { imageUrl } from '$lib/media/api';
	import { creditCaption, initials, type Credit } from '$lib/media/people';

	interface Props {
		credit: Credit;
		token: string;
	}

	let { credit, token }: Props = $props();

	let imageFailed = $state(false);
	const caption = $derived(creditCaption(credit));
</script>

<a
	href={resolve('/person/[id]', { id: credit.person_id })}
	class="group block w-[104px] shrink-0 outline-none sm:w-[124px] md:w-[140px]"
>
	<div
		class="relative aspect-[2/3] overflow-hidden rounded-md bg-surface ring-gold transition-[transform,box-shadow] duration-200 group-hover:shadow-[0_10px_30px_rgb(0_0_0/0.6)] group-focus-visible:scale-[1.07] group-focus-visible:shadow-[0_10px_30px_rgb(0_0_0/0.7)] group-focus-visible:ring-[3px]"
	>
		{#if credit.image_url && !imageFailed}
			<img
				src={imageUrl(credit.image_url, token)}
				alt=""
				loading="lazy"
				onerror={() => (imageFailed = true)}
				class="size-full object-cover transition-transform duration-300 group-hover:scale-105"
			/>
		{:else}
			<span
				class="absolute inset-0 grid place-items-center text-2xl font-semibold text-neutral-500"
				aria-hidden="true">{initials(credit.name)}</span
			>
		{/if}
	</div>
	<p class="mt-2 truncate text-sm font-medium text-neutral-100 group-focus-visible:text-gold">
		{credit.name}
	</p>
	{#if caption.text}
		<p class="truncate text-xs text-neutral-400">{caption.text}</p>
	{:else if caption.roleKey}
		<p class="truncate text-xs text-neutral-400">{t(caption.roleKey)}</p>
	{/if}
</a>
