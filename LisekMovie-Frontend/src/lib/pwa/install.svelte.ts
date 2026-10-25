// "Install app" (Faza 6, PWA). Chrome, Edge and Android fire
// `beforeinstallprompt` once the page qualifies (manifest, service worker,
// HTTPS); the event is kept and replayed when the user asks. iPhone/iPad
// Safari has no prompt: the menu shows how to add the app to the Home
// Screen instead. Nothing is offered when already running installed.

interface InstallPromptEvent extends Event {
	prompt(): Promise<void>;
	userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
}

export const install = $state({
	/** A deferred browser prompt is ready. */
	available: false,
	/** Safari on iOS/iPadOS, not yet on the Home Screen. */
	ios: false
});

let deferred: InstallPromptEvent | null = null;
let started = false;

export function standalone(): boolean {
	return (
		window.matchMedia?.('(display-mode: standalone)').matches ||
		(navigator as Navigator & { standalone?: boolean }).standalone === true
	);
}

/** iPhone, iPod, or an iPad (which reports itself as a Mac with touch). */
export function isIos(userAgent: string, touchPoints: number): boolean {
	return /iPhone|iPad|iPod/.test(userAgent) || (/Macintosh/.test(userAgent) && touchPoints > 1);
}

/** Starts listening; call once in the browser (root layout). */
export function watchInstall(): void {
	if (started || typeof window === 'undefined') return;
	started = true;
	if (standalone()) return;
	install.ios = isIos(navigator.userAgent, navigator.maxTouchPoints ?? 0);
	window.addEventListener('beforeinstallprompt', (event) => {
		event.preventDefault();
		deferred = event as InstallPromptEvent;
		install.available = true;
	});
	window.addEventListener('appinstalled', () => {
		deferred = null;
		install.available = false;
		install.ios = false;
	});
}

/** Shows the browser's install dialog; `true` when the user accepted. */
export async function promptInstall(): Promise<boolean> {
	if (!deferred) return false;
	const event = deferred;
	deferred = null;
	install.available = false;
	await event.prompt();
	return (await event.userChoice).outcome === 'accepted';
}
