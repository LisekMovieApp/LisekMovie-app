<!--
  Faza 20: one picture in the metadata editor (poster, backdrop, logo or an
  episode still). Shows the current image; a new one comes from a file
  (button or drag and drop) or from a link. The server decodes and
  re-encodes it, so only a clean picture is stored, and locks the kind
  against provider refreshes.
-->
<script lang="ts">
	import { onMount } from 'svelte';
	import { t } from '$lib/i18n';
	import { getSessionCookieClientSide } from '$lib/auth';
	import { editorErrorKey, setArtwork, type ArtworkKind } from '$lib/admin/mediaItems';
	import Icon from './Icon.svelte';

	interface Props {
		itemId: string;
		kind: ArtworkKind;
		/** Cache-buster of the image as loaded with the editor. */
		version: number;
		onchanged?: (kind: ArtworkKind) => void;
	}

	let { itemId, kind, version, onchanged }: Props = $props();

	const MAX_BYTES = 25 * 1024 * 1024;
	const ASPECT: Record<ArtworkKind, string> = {
		poster: 'aspect-[2/3]',
		backdrop: 'aspect-video',
		logo: 'aspect-[3/1]',
		thumb: 'aspect-video'
	};

	let token = $state('');
	let src = $state<string | null>(null);
	let missing = $state(false);
	let busy = $state(false);
	let errorKey = $state<string | null>(null);
	let done = $state(false);
	let url = $state('');
	let dragging = $state(false);
	let fileInput: HTMLInputElement | undefined = $state();

	function withToken(path: string) {
		return `${path}${path.includes('?') ? '&' : '?'}token=${encodeURIComponent(token)}`;
	}

	onMount(() => {
		token = getSessionCookieClientSide() ?? '';
		src = withToken(`/api/media/${encodeURIComponent(itemId)}/${kind}?v=${version}`);
	});

	async function send(source: Blob | string) {
		if (busy) return;
		busy = true;
		errorKey = null;
		done = false;
		try {
			const next = await setArtwork(token, itemId, kind, source);
			src = withToken(next);
			missing = false;
			done = true;
			url = '';
			onchanged?.(kind);
		} catch (error) {
			errorKey = editorErrorKey(error, 'admin.editor_errors.artwork_failed');
		} finally {
			busy = false;
		}
	}

	function pick(file: File | undefined) {
		if (!file) return;
		if (!file.type.startsWith('image/')) {
			errorKey = 'admin.editor_errors.artwork_invalid_image';
			return;
		}
		if (file.size > MAX_BYTES) {
			errorKey = 'admin.editor_errors.artwork_too_large';
			return;
		}
		void send(file);
	}

	function onDrop(event: DragEvent) {
		event.preventDefault();
		dragging = false;
		pick(event.dataTransfer?.files?.[0]);
	}

	function submitUrl() {
		const value = url.trim();
		if (!/^https?:\/\/\S+$/i.test(value)) {
			errorKey = 'admin.editor_errors.artwork_invalid_url';
			return;
		}
		void send(value);
	}
</script>

<section
	class="rounded-lg border border-white/10 bg-black/30 p-3"
	aria-label={t(`admin.editor_art.${kind}`)}
>
	<div class="mb-2 flex items-center justify-between gap-2">
		<h4 class="text-sm font-semibold text-neutral-100">{t(`admin.editor_art.${kind}`)}</h4>
		{#if done}<span role="status" class="flex items-center gap-1 text-xs text-emerald-400"
				><Icon name="check" class="size-3.5" />{t('admin.editor_art.saved')}</span
			>{/if}
	</div>
	<button
		type="button"
		class="relative flex w-full items-center justify-center overflow-hidden rounded-md border border-dashed transition-colors {ASPECT[
			kind
		]} {dragging
			? 'border-gold bg-gold/10'
			: 'border-white/15 bg-neutral-900 hover:border-gold-dim'}"
		aria-label={t('admin.editor_art.upload')}
		disabled={busy}
		onclick={() => fileInput?.click()}
		ondragover={(event) => {
			event.preventDefault();
			dragging = true;
		}}
		ondragleave={() => (dragging = false)}
		ondrop={onDrop}
	>
		{#if src && !missing}
			<img
				{src}
				alt=""
				class="size-full {kind === 'logo' ? 'object-contain p-2' : 'object-cover'}"
				onerror={() => (missing = true)}
			/>
		{:else}
			<span class="flex flex-col items-center gap-1 p-2 text-center text-xs text-neutral-500">
				<Icon name="camera" class="size-6" />
				{t('admin.editor_art.empty')}
			</span>
		{/if}
		{#if busy}
			<span class="absolute inset-0 grid place-items-center bg-black/60">
				<span class="size-6 animate-spin rounded-full border-2 border-white/30 border-t-white"
				></span>
			</span>
		{/if}
	</button>
	<input
		bind:this={fileInput}
		type="file"
		accept="image/jpeg,image/png,image/webp,image/gif"
		class="hidden"
		aria-label={t('admin.editor_art.upload')}
		onchange={(event) => {
			const input = event.currentTarget as HTMLInputElement;
			pick(input.files?.[0]);
			input.value = '';
		}}
	/>
	<div class="mt-2 flex gap-1.5">
		<input
			type="url"
			bind:value={url}
			placeholder={t('admin.editor_art.url_placeholder')}
			aria-label={t('admin.editor_art.url_placeholder')}
			disabled={busy}
			class="min-w-0 flex-1 rounded-md border border-neutral-700 bg-black px-2 py-1.5 text-xs text-neutral-100 focus:border-gold"
			onkeydown={(event) => {
				if (event.key === 'Enter') {
					event.preventDefault();
					submitUrl();
				}
			}}
		/>
		<button
			type="button"
			disabled={busy || !url.trim()}
			onclick={submitUrl}
			class="shrink-0 rounded-md border border-neutral-700 px-2 py-1.5 text-xs text-neutral-200 hover:border-gold-dim disabled:opacity-50"
		>
			{t('admin.editor_art.use_url')}
		</button>
	</div>
	{#if errorKey}<p role="alert" class="mt-1.5 text-xs text-red-400">{t(errorKey)}</p>{/if}
</section>
