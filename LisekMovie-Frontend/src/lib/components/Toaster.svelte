<!--
  The toasts of `$lib/toast` (Faza 17), bottom right (bottom, full width on
  phones). Errors are announced assertively and stay until closed.
-->
<script lang="ts">
	import { t } from '$lib/i18n';
	import { dismissToast, toasts } from '$lib/toast/toast.svelte';

	const STYLE = {
		success: 'border-gold/40 bg-surface text-neutral-100',
		warning: 'border-amber-400/50 bg-amber-950/90 text-amber-100',
		error: 'border-red-500/50 bg-red-950/95 text-red-100'
	} as const;
</script>

<div
	class="pointer-events-none fixed inset-x-4 bottom-4 z-[60] flex flex-col items-stretch gap-2 sm:inset-x-auto sm:right-4 sm:w-96"
>
	{#each toasts as toast (toast.id)}
		<div
			role={toast.kind === 'error' ? 'alert' : 'status'}
			aria-live={toast.kind === 'error' ? 'assertive' : 'polite'}
			class="pointer-events-auto flex items-start gap-3 rounded-lg border p-3 text-sm shadow-lg shadow-black/40 {STYLE[
				toast.kind
			]}"
		>
			<span aria-hidden="true" class="mt-0.5">
				{toast.kind === 'success' ? '✓' : toast.kind === 'warning' ? '!' : '✕'}
			</span>
			<p class="min-w-0 flex-1 break-words">{toast.message}</p>
			<button
				type="button"
				aria-label={t('toast.close')}
				onclick={() => dismissToast(toast.id)}
				class="-m-1 rounded p-1 opacity-70 hover:opacity-100"
			>
				✕
			</button>
		</div>
	{/each}
</div>
