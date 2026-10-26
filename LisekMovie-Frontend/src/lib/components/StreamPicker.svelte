<!--
  "Wideo / Dźwięk / Napisy" before playback (Faza 9 for films, Faza 10 for
  episodes too): the file's picture as one line, its audio tracks and
  subtitles as dropdowns. The picks are bound to the parent, which puts
  them into the `/watch/[id]?audio=&sub=` link of its Play button. Native
  `<select>`s: a TV remote's arrows move between them (spatial navigation
  leaves their value alone) and OK opens the picker.
-->
<script lang="ts">
	import { getLocale, t } from '$lib/i18n';
	import { audioOptionLabel, subtitleOptionLabel, type ItemDetails } from '$lib/media/details';

	interface Props {
		details: ItemDetails;
		audio: number | null;
		subtitle: string;
		/** Unique per page: the dropdowns' ids. */
		idPrefix?: string;
	}

	let { details, audio = $bindable(), subtitle = $bindable(), idPrefix = 'pick' }: Props = $props();

	const words = $derived({
		unknown: t('player.unknown_language'),
		default: t('details.default_track'),
		forced: t('player.forced')
	});
</script>

{#if details.video || details.audio_tracks.length > 0}
	<dl
		class="grid max-w-2xl grid-cols-[5.5rem_1fr] items-center gap-x-4 gap-y-2.5 text-sm md:grid-cols-[6.5rem_1fr]"
	>
		{#if details.video}
			<dt class="text-neutral-300">{t('details.video')}</dt>
			<dd class="px-1 text-neutral-200">{details.video.label}</dd>
		{/if}
		{#if details.audio_tracks.length > 0}
			<dt class="text-neutral-300">
				<label for="{idPrefix}-audio">{t('details.audio')}</label>
			</dt>
			<dd>
				<select
					id="{idPrefix}-audio"
					class="pick"
					value={audio === null ? '' : String(audio)}
					onchange={(e) => {
						const v = (e.currentTarget as HTMLSelectElement).value;
						audio = v === '' ? null : Number(v);
					}}
				>
					{#each details.audio_tracks as track (track.index)}
						<option value={String(track.index)}
							>{audioOptionLabel(track, getLocale(), words)}</option
						>
					{/each}
				</select>
			</dd>
		{/if}
		<dt class="text-neutral-300">
			<label for="{idPrefix}-subtitle">{t('details.subtitles')}</label>
		</dt>
		<dd>
			<select id="{idPrefix}-subtitle" class="pick" bind:value={subtitle}>
				<option value="">{t('details.subtitles_off')}</option>
				{#each details.subtitle_tracks as track (track.id)}
					<option value={track.id}>{subtitleOptionLabel(track, getLocale(), words)}</option>
				{/each}
			</select>
		</dd>
	</dl>
{/if}

<style>
	.pick {
		width: 100%;
		border: 1px solid rgb(255 255 255 / 0.12);
		border-radius: 0.35rem;
		background: #1c1c1c;
		padding: 0.55rem 0.7rem;
		color: #f5f5f5;
	}
	.pick:hover {
		border-color: rgb(255 255 255 / 0.3);
	}
	.pick:focus-visible {
		outline: 2px solid #d5ad42;
		outline-offset: 2px;
	}
</style>
