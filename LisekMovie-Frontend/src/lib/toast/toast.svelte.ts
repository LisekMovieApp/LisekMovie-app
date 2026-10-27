// Short notices in the corner of the screen (Faza 17): a saved Fix match,
// a failed one, a partial success. `Toaster.svelte` (in the root layout)
// renders them; anything can raise one with `showToast`.

export type ToastKind = 'success' | 'error' | 'warning';

export interface Toast {
	id: number;
	kind: ToastKind;
	message: string;
}

/** Errors stay until closed; the rest go after this many milliseconds. */
export const TOAST_TIMEOUT = 6000;

export const toasts = $state<Toast[]>([]);
let nextId = 1;

export function dismissToast(id: number): void {
	const index = toasts.findIndex((toast) => toast.id === id);
	if (index >= 0) toasts.splice(index, 1);
}

export function showToast(
	kind: ToastKind,
	message: string,
	timeout = kind === 'error' ? 0 : TOAST_TIMEOUT
): number {
	const id = nextId++;
	toasts.push({ id, kind, message });
	// At most five at a time: the oldest go first.
	while (toasts.length > 5) toasts.shift();
	if (timeout > 0) setTimeout(() => dismissToast(id), timeout);
	return id;
}
