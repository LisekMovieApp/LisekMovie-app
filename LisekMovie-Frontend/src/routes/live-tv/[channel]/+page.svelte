<!--
  Faza 20 — the live TV player. Asks the server to start the channel (an
  FFmpeg process writing a live HLS playlist) and plays it with hls.js (or
  natively in Safari). When the browser cannot decode the channel's video
  it asks once more with transcoding (H.264). The channel list on the side
  switches channels in place; leaving stops the stream on the server.
-->
<script lang="ts">
	import Hls from 'hls.js';
	import { onMount, untrack } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { getLocale, t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import {
		fetchChannels,
		LiveTvError,
		logoSrc,
		playChannel,
		progressOf,
		stopLive,
		type ChannelNow,
		type LiveSession
	} from '$lib/livetv/api';
	import Icon from '$lib/components/Icon.svelte';

	const channelId = $derived(page.params.channel ?? '');

	let token = $state('');
	let video = $state<HTMLVideoElement>();
	let root = $state<HTMLDivElement>();
	let channels = $state<ChannelNow[]>([]);
	let session = $state<LiveSession | null>(null);
	let status = $state<'starting' | 'playing' | 'error'>('starting');
	let errorKey = $state<string | null>(null);
	let paused = $state(false);
	let muted = $state(false);
	let listOpen = $state(false);
	let controls = $state(true);
	let now = $state(Date.now());
	let hls: Hls | null = null;
	let attempt = 0;
	let hideTimer: ReturnType<typeof setTimeout> | undefined;

	const channel = $derived(channels.find((c) => c.id === channelId) ?? null);

	function clock(iso: string) {
		return new Intl.DateTimeFormat(getLocale(), { hour: '2-digit', minute: '2-digit' }).format(
			new Date(iso)
		);
	}

	function poke() {
		controls = true;
		clearTimeout(hideTimer);
		hideTimer = setTimeout(() => {
			if (!paused && !listOpen) controls = false;
		}, 3500);
	}

	function teardown() {
		hls?.destroy();
		hls = null;
		if (video) {
			video.removeAttribute('src');
			video.load();
		}
		if (session) void stopLive(token, session.session_id, true);
		session = null;
	}

	async function start(id: string, transcode = false) {
		const mine = ++attempt;
		teardown();
		status = 'starting';
		errorKey = null;
		let next: LiveSession;
		try {
			next = await playChannel(token, id, transcode);
		} catch (error) {
			if (mine !== attempt) return;
			status = 'error';
			const code = error instanceof LiveTvError ? error.code : 'network_error';
			errorKey = [
				'live_stream_unavailable',
				'live_capacity_reached',
				'transcode_unavailable',
				'channel_not_found'
			].includes(code)
				? `livetv.errors.${code}`
				: 'livetv.errors.failed';
			return;
		}
		if (mine !== attempt) {
			void stopLive(token, next.session_id);
			return;
		}
		session = next;
		const element = video;
		if (!element) return;
		if (Hls.isSupported()) {
			const instance = new Hls({
				liveSyncDurationCount: 3,
				lowLatencyMode: false,
				backBufferLength: 30
			});
			hls = instance;
			instance.on(Hls.Events.ERROR, (_event, data) => {
				if (instance !== hls || !data.fatal) return;
				// A codec the browser cannot play: once more, transcoded.
				if (data.type === Hls.ErrorTypes.MEDIA_ERROR && !next.transcoding) {
					void start(id, true);
				} else if (data.type === Hls.ErrorTypes.NETWORK_ERROR) {
					instance.startLoad();
				} else {
					status = 'error';
					errorKey = 'livetv.errors.failed';
				}
			});
			instance.loadSource(next.stream_url);
			instance.attachMedia(element);
		} else {
			element.src = next.stream_url;
		}
		element.play().catch(() => (paused = true));
	}

	function togglePlay() {
		if (!video) return;
		if (video.paused) void video.play().catch(() => {});
		else video.pause();
		poke();
	}

	function toggleFullscreen() {
		if (document.fullscreenElement) void document.exitFullscreen();
		else void root?.requestFullscreen?.().catch(() => {});
	}

	function zap(step: number) {
		if (channels.length === 0) return;
		const index = channels.findIndex((c) => c.id === channelId);
		const next = channels[(index + step + channels.length) % channels.length];
		void goto(resolve('/live-tv/[channel]', { channel: next.id }), { replaceState: true });
	}

	function onKey(event: KeyboardEvent) {
		if (event.key === 'Escape' && listOpen) listOpen = false;
		else if (event.key === ' ' || event.key === 'k') {
			event.preventDefault();
			togglePlay();
		} else if (event.key === 'f') toggleFullscreen();
		else if (event.key === 'm' && video) video.muted = !video.muted;
		else if (event.key === 'ChannelUp' || event.key === 'PageUp') zap(-1);
		else if (event.key === 'ChannelDown' || event.key === 'PageDown') zap(1);
		poke();
	}

	// Only the channel and the session token restart the stream; what
	// `start` reads itself must not (it would restart in a loop).
	$effect(() => {
		const id = channelId;
		const ready = token !== '';
		if (ready && id) untrack(() => void start(id));
	});

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		fetchChannels(token)
			.then((list) => (channels = list))
			.catch(() => {});
		const clockTimer = setInterval(() => (now = Date.now()), 30_000);
		const refresh = setInterval(() => {
			fetchChannels(token)
				.then((list) => (channels = list))
				.catch(() => {});
		}, 5 * 60_000);
		const onLeave = () => session && void stopLive(token, session.session_id, true);
		window.addEventListener('pagehide', onLeave);
		poke();
		return () => {
			attempt++;
			clearInterval(clockTimer);
			clearInterval(refresh);
			clearTimeout(hideTimer);
			window.removeEventListener('pagehide', onLeave);
			teardown();
		};
	});
</script>

<svelte:head><title>{channel?.name ?? t('livetv.title')} · LisekMovie</title></svelte:head>
<svelte:window onkeydown={onKey} />

<div
	bind:this={root}
	class="fixed inset-0 z-[60] bg-black text-white {controls ? '' : 'cursor-none'}"
	role="application"
	aria-label={t('livetv.player')}
	onpointermove={poke}
>
	<video
		bind:this={video}
		class="absolute inset-0 size-full object-contain"
		playsinline
		autoplay
		onclick={togglePlay}
		onplaying={() => {
			status = 'playing';
			paused = false;
		}}
		onpause={() => (paused = true)}
		onplay={() => (paused = false)}
		onvolumechange={() => (muted = video?.muted ?? false)}
	></video>

	{#if status === 'starting'}
		<div class="absolute inset-0 grid place-items-center" role="status">
			<div class="flex flex-col items-center gap-3 text-neutral-300">
				<span class="size-10 animate-spin rounded-full border-4 border-white/20 border-t-gold"
				></span>
				{t('livetv.tuning', { name: channel?.name ?? '' })}
			</div>
		</div>
	{:else if status === 'error'}
		<div class="absolute inset-0 grid place-items-center p-6 text-center">
			<div>
				<p class="text-lg text-red-300" role="alert">{t(errorKey ?? 'livetv.errors.failed')}</p>
				<div class="mt-4 flex justify-center gap-2">
					<button
						type="button"
						class="rounded-md bg-gold px-4 py-2 font-semibold text-black"
						onclick={() => start(channelId, true)}>{t('livetv.retry_compatible')}</button
					>
					<a href={resolve('/live-tv')} class="rounded-md px-4 py-2 ring-1 ring-white/20"
						>{t('livetv.back_to_guide')}</a
					>
				</div>
			</div>
		</div>
	{/if}

	<div
		class="pointer-events-none absolute inset-0 flex flex-col justify-between transition-opacity duration-300 {controls
			? 'opacity-100'
			: 'opacity-0'}"
	>
		<div
			class="pointer-events-auto flex items-center gap-3 bg-gradient-to-b from-black/85 to-transparent px-4 pt-4 pb-16 md:px-8"
		>
			<a
				href={resolve('/live-tv')}
				aria-label={t('livetv.back_to_guide')}
				class="grid size-11 place-items-center rounded-full hover:bg-white/10"
			>
				<Icon name="back" class="size-7" />
			</a>
			{#if channel?.logo_url}
				<img src={logoSrc(channel, token)} alt="" class="h-10 w-14 object-contain" />
			{/if}
			<div class="min-w-0">
				<p class="flex items-center gap-2 truncate text-lg font-bold md:text-2xl">
					<span class="rounded bg-red-600 px-1.5 py-0.5 text-[0.65rem] font-bold tracking-wider"
						>{t('livetv.live')}</span
					>
					{channel?.name ?? session?.name ?? ''}
				</p>
				{#if channel?.now}
					<p class="truncate text-sm text-neutral-300">
						{channel.now.title} · {clock(channel.now.starts_at)}–{clock(channel.now.ends_at)}
					</p>
				{/if}
			</div>
		</div>

		<div
			class="pointer-events-auto bg-gradient-to-t from-black/90 to-transparent px-4 pt-16 pb-5 md:px-8"
		>
			{#if channel?.now}
				<div class="mb-3 h-1 overflow-hidden rounded-full bg-white/15">
					<div class="h-full bg-gold" style:width={`${progressOf(channel.now, now) * 100}%`}></div>
				</div>
			{/if}
			<div class="flex items-center gap-2">
				<button
					type="button"
					class="ctl"
					aria-label={paused ? t('player.play') : t('player.pause')}
					onclick={togglePlay}
				>
					<Icon name={paused ? 'play' : 'pause'} class="size-7" />
				</button>
				<button
					type="button"
					class="ctl"
					aria-label={t('livetv.previous_channel')}
					onclick={() => zap(-1)}
				>
					<Icon name="chevron_left" class="size-6" />
				</button>
				<button
					type="button"
					class="ctl"
					aria-label={t('livetv.next_channel')}
					onclick={() => zap(1)}
				>
					<Icon name="chevron_right" class="size-6" />
				</button>
				<button
					type="button"
					class="ctl"
					aria-label={muted ? t('player.unmute') : t('player.mute')}
					onclick={() => video && (video.muted = !video.muted)}
				>
					<Icon name={muted ? 'muted' : 'volume'} class="size-6" />
				</button>
				{#if channel?.next}
					<p class="ml-2 hidden min-w-0 truncate text-sm text-neutral-400 md:block">
						{t('livetv.next_up', {
							time: clock(channel.next.starts_at),
							title: channel.next.title
						})}
					</p>
				{/if}
				<div class="ml-auto flex items-center gap-2">
					<button
						type="button"
						class="ctl"
						aria-label={t('livetv.channels')}
						aria-expanded={listOpen}
						onclick={() => (listOpen = !listOpen)}
					>
						<Icon name="menu" class="size-6" />
					</button>
					<button
						type="button"
						class="ctl"
						aria-label={t('player.fullscreen')}
						onclick={toggleFullscreen}
					>
						<Icon name="fullscreen" class="size-6" />
					</button>
				</div>
			</div>
		</div>
	</div>

	{#if listOpen}
		<aside
			class="absolute top-0 right-0 bottom-0 z-10 flex w-full max-w-sm flex-col bg-neutral-950/95 backdrop-blur-xl"
			aria-label={t('livetv.channels')}
		>
			<div class="flex items-center justify-between border-b border-white/10 px-4 py-3">
				<h2 class="font-semibold">{t('livetv.channels')}</h2>
				<button
					type="button"
					class="ctl"
					aria-label={t('player.close')}
					onclick={() => (listOpen = false)}
				>
					<Icon name="close" class="size-5" />
				</button>
			</div>
			<ul class="flex-1 overflow-y-auto p-2">
				{#each channels as item (item.id)}
					<li>
						<a
							href={resolve('/live-tv/[channel]', { channel: item.id })}
							data-sveltekit-replacestate
							aria-current={item.id === channelId ? 'true' : undefined}
							class="flex items-center gap-3 rounded-lg px-3 py-2 hover:bg-white/10 aria-[current=true]:bg-gold/15"
						>
							<span class="w-8 shrink-0 text-right text-xs text-neutral-500 tabular-nums"
								>{item.number ?? ''}</span
							>
							<span class="min-w-0">
								<span class="block truncate text-sm font-medium">{item.name}</span>
								{#if item.now}<span class="block truncate text-xs text-neutral-400"
										>{item.now.title}</span
									>{/if}
							</span>
						</a>
					</li>
				{/each}
			</ul>
		</aside>
	{/if}
</div>

<style>
	.ctl {
		display: grid;
		place-items: center;
		width: 2.75rem;
		height: 2.75rem;
		border-radius: 9999px;
	}
	.ctl:hover,
	.ctl:focus-visible {
		background: rgb(255 255 255 / 0.12);
	}
</style>
