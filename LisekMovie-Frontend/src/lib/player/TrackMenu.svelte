<!--
  The player's side panel: audio and subtitles side by side (the VOD
  layout), or playback settings with technical details. Choices are
  reported up; the player decides whether they switch in place or start
  a new session. Works with a remote: it is a navigation scope, opens on
  the selected option and Back closes it (handled by the player).
-->
<script lang="ts" module>
	export interface PlaybackStats {
		bandwidthKbps: number | null;
		droppedFrames: number | null;
		bufferSeconds: number;
		width: number;
		height: number;
	}
</script>

<script lang="ts">
	import { fly } from 'svelte/transition';
	import { getLocale, t } from '$lib/i18n';
	import Icon from '$lib/components/Icon.svelte';
	import { navGroup, navScope } from '$lib/spatial/spatial';
	import type { DownmixPreference, PlaybackSession } from '$lib/playback/api';
	import { audioTrackLabel, subtitleTrackLabel } from './labels';
	import type { SubtitleSize } from './preferences';

	interface Props {
		view: 'tracks' | 'settings';
		session: PlaybackSession;
		audio: number | null;
		subtitle: string;
		burnIn: boolean;
		downmix: DownmixPreference;
		compatible: boolean;
		subtitleSize: SubtitleSize;
		busy: boolean;
		stats: PlaybackStats;
		onaudio: (index: number) => void;
		onsubtitle: (id: string) => void;
		ondownmix: (mode: DownmixPreference) => void;
		oncompatible: (value: boolean) => void;
		onburnin: (value: boolean) => void;
		onsize: (size: SubtitleSize) => void;
		onclose: () => void;
	}

	let {
		view,
		session,
		audio,
		subtitle,
		burnIn,
		downmix,
		compatible,
		subtitleSize,
		busy,
		stats,
		onaudio,
		onsubtitle,
		ondownmix,
		oncompatible,
		onburnin,
		onsize,
		onclose
	}: Props = $props();

	const DOWNMIX: DownmixPreference[] = ['auto', 'standard', 'dialogue', 'night', 'pro_logic_ii'];
	const SIZES: SubtitleSize[] = ['small', 'medium', 'large'];

	const locale = $derived(getLocale());
	const reduced =
		typeof window !== 'undefined' &&
		window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;

	const option =
		'flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors hover:bg-white/10 focus-visible:bg-white/15 aria-checked:text-white';
	const heading = 'mb-2 px-3 text-xs font-semibold tracking-[0.12em] text-neutral-400 uppercase';
</script>

{#snippet check(on: boolean)}
	<Icon name="check" class="size-5 shrink-0 {on ? 'text-gold' : 'invisible'}" />
{/snippet}

{#snippet toggle(label: string, value: boolean, change: (value: boolean) => void)}
	<button
		type="button"
		role="switch"
		aria-checked={value}
		class="flex w-full items-center justify-between gap-4 rounded-lg px-3 py-3 text-left hover:bg-white/10 focus-visible:bg-white/15"
		onclick={() => change(!value)}
	>
		<span>{label}</span>
		<span
			class="relative h-6 w-11 shrink-0 rounded-full transition-colors {value
				? 'bg-gold'
				: 'bg-white/25'}"
			aria-hidden="true"
		>
			<span
				class="absolute top-0.5 left-0.5 size-5 rounded-full bg-white shadow transition-transform {value
					? 'translate-x-5'
					: ''}"
			></span>
		</span>
	</button>
{/snippet}

<div
	class="absolute inset-x-0 bottom-0 z-30 flex max-h-[85vh] flex-col rounded-t-2xl bg-neutral-950/95 text-neutral-300 shadow-2xl ring-1 ring-white/10 backdrop-blur-xl md:inset-x-auto md:right-6 md:bottom-28 md:max-h-[min(70vh,40rem)] md:w-[38rem] md:rounded-2xl"
	role="dialog"
	aria-label={view === 'tracks' ? t('player.tracks') : t('player.settings')}
	use:navScope={{ autofocus: true }}
	transition:fly={{ y: 24, duration: reduced ? 0 : 180 }}
>
	<header class="flex items-center justify-between gap-4 border-b border-white/10 px-5 py-3">
		<h2 class="text-lg font-semibold text-white">
			{view === 'tracks' ? t('player.tracks') : t('player.settings')}
		</h2>
		<div class="flex items-center gap-3">
			{#if busy}
				<span class="flex items-center gap-2 text-sm text-gold" role="status">
					<span class="size-4 animate-spin rounded-full border-2 border-gold/30 border-t-gold"
					></span>
					{t('player.switching')}
				</span>
			{/if}
			<button
				type="button"
				class="grid size-10 place-items-center rounded-full hover:bg-white/10 focus-visible:bg-white/15"
				aria-label={t('player.close')}
				onclick={onclose}
			>
				<Icon name="close" class="size-5" />
			</button>
		</div>
	</header>

	<div class="overflow-y-auto overscroll-contain p-3 md:p-4">
		{#if view === 'tracks'}
			<div class="grid gap-4 md:grid-cols-2">
				<section use:navGroup>
					<h3 class={heading}>{t('player.audio_track')}</h3>
					<div role="radiogroup" aria-label={t('player.audio_track')}>
						{#each session.audio_tracks as track (track.index)}
							{@const label = audioTrackLabel(track, locale, t('player.unknown_language'))}
							{@const on = audio === track.index}
							<button
								type="button"
								role="radio"
								aria-checked={on}
								class={option}
								data-nav-default={on ? '' : undefined}
								onclick={() => onaudio(track.index)}
							>
								{@render check(on)}
								<span class="min-w-0">
									<span class="block truncate {on ? 'font-semibold' : ''}">{label.name}</span>
									{#if label.detail}<span class="block text-xs text-neutral-500"
											>{label.detail}</span
										>{/if}
								</span>
							</button>
						{:else}
							<p class="px-3 py-2 text-sm text-neutral-500">{t('player.no_tracks')}</p>
						{/each}
					</div>
				</section>
				<section use:navGroup>
					<h3 class={heading}>{t('player.subtitles')}</h3>
					<div role="radiogroup" aria-label={t('player.subtitles')}>
						<button
							type="button"
							role="radio"
							aria-checked={subtitle === ''}
							class={option}
							onclick={() => onsubtitle('')}
						>
							{@render check(subtitle === '')}
							<span class={subtitle === '' ? 'font-semibold' : ''}>{t('player.subtitles_off')}</span
							>
						</button>
						{#each session.subtitle_tracks as track (track.id)}
							{@const label = subtitleTrackLabel(
								track,
								locale,
								t('player.unknown_language'),
								t('player.forced')
							)}
							{@const on = subtitle === track.id}
							<button
								type="button"
								role="radio"
								aria-checked={on}
								class={option}
								onclick={() => onsubtitle(track.id)}
							>
								{@render check(on)}
								<span class="min-w-0">
									<span class="block truncate {on ? 'font-semibold' : ''}">{label.name}</span>
									<span class="block text-xs text-neutral-500">
										{label.detail}{track.kind === 'image' ? ` · ${t('player.burnt_in')}` : ''}
									</span>
								</span>
							</button>
						{/each}
					</div>
				</section>
			</div>
		{:else}
			<div class="grid gap-5">
				<section use:navGroup>
					<h3 class={heading}>{t('player.downmix')}</h3>
					<p class="mb-2 px-3 text-xs text-neutral-500">{t('player.downmix_hint')}</p>
					<div role="radiogroup" aria-label={t('player.downmix')} class="grid sm:grid-cols-2">
						{#each DOWNMIX as mode (mode)}
							<button
								type="button"
								role="radio"
								aria-checked={downmix === mode}
								class={option}
								data-nav-default={downmix === mode ? '' : undefined}
								onclick={() => ondownmix(mode)}
							>
								{@render check(downmix === mode)}
								<span>{t(`player.downmix_modes.${mode}`)}</span>
							</button>
						{/each}
					</div>
				</section>

				<section use:navGroup>
					<h3 class={heading}>{t('player.subtitle_size')}</h3>
					<div
						role="radiogroup"
						aria-label={t('player.subtitle_size')}
						class="grid grid-cols-3 gap-2 px-3"
					>
						{#each SIZES as size, i (size)}
							<button
								type="button"
								role="radio"
								aria-checked={subtitleSize === size}
								class="rounded-lg py-2 font-semibold ring-1 transition-colors focus-visible:bg-white/15 {subtitleSize ===
								size
									? 'bg-white text-black ring-white'
									: 'ring-white/20 hover:bg-white/10'}"
								onclick={() => onsize(size)}
							>
								<span aria-hidden="true" style:font-size={`${0.85 + i * 0.25}rem`}
									>{t('player.subtitle_sample')}</span
								>
								<span class="sr-only">{t(`player.subtitle_sizes.${size}`)}</span>
							</button>
						{/each}
					</div>
				</section>

				<section use:navGroup class="grid">
					{@render toggle(t('player.compatible'), compatible, oncompatible)}
					{@render toggle(t('player.burn_in'), burnIn, onburnin)}
				</section>

				<section class="rounded-xl bg-white/5 p-4 text-sm">
					<h3 class="mb-3 text-xs font-semibold tracking-[0.12em] text-neutral-400 uppercase">
						{t('player.diagnostics')}
					</h3>
					<dl class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5">
						<dt class="text-neutral-500">{t('player.diag_mode')}</dt>
						<dd class="text-white">{t(`player.modes.${session.mode}`)}</dd>
						<dt class="text-neutral-500">{t('player.diag_video')}</dt>
						<dd class="text-white tabular-nums">
							{session.delivered.video_codec.toUpperCase()}
							{stats.width || session.delivered.width}×{stats.height || session.delivered.height}
							{#if session.delivered.tone_mapped}· {t('player.tone_mapped')}{/if}
						</dd>
						{#if session.delivered.audio_codec}
							<dt class="text-neutral-500">{t('player.diag_audio')}</dt>
							<dd class="text-white">
								{session.delivered.audio_codec.toUpperCase()}
								{t('player.channels', { count: session.delivered.audio_channels ?? 2 })}
								{#if session.delivered.downmix}· {t(
										`player.downmix_modes.${session.delivered.downmix}`
									)}{/if}
							</dd>
						{/if}
						{#if stats.bandwidthKbps}
							<dt class="text-neutral-500">{t('player.diag_bandwidth')}</dt>
							<dd class="text-white tabular-nums">
								{t('player.mbps', { value: (stats.bandwidthKbps / 1000).toFixed(1) })}
							</dd>
						{/if}
						<dt class="text-neutral-500">{t('player.diag_buffer')}</dt>
						<dd class="text-white tabular-nums">
							{t('player.seconds', { value: stats.bufferSeconds.toFixed(1) })}
						</dd>
						{#if stats.droppedFrames !== null}
							<dt class="text-neutral-500">{t('player.diag_dropped')}</dt>
							<dd class="text-white tabular-nums">{stats.droppedFrames}</dd>
						{/if}
						{#if session.reasons.length}
							<dt class="text-neutral-500">{t('player.reasons_label')}</dt>
							<dd class="text-white">
								{session.reasons.map((r) => t(`player.reasons.${r}`)).join(', ')}
							</dd>
						{/if}
					</dl>
				</section>
			</div>
		{/if}
	</div>
</div>
