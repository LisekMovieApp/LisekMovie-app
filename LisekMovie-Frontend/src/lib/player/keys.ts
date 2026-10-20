/**
 * What a key press means to the player: keyboard shortcuts (YouTube /
 * Netflix style) and TV remote media keys, by `key` and by the numeric
 * codes Tizen and webOS still send.
 */

export type PlayerCommand =
	| 'toggle'
	| 'play'
	| 'pause'
	| 'stop'
	| 'rewind'
	| 'forward'
	| 'rewind_long'
	| 'forward_long'
	| 'volume_up'
	| 'volume_down'
	| 'mute'
	| 'fullscreen'
	| 'subtitles'
	| 'back';

const BY_KEY: Record<string, PlayerCommand> = {
	' ': 'toggle',
	k: 'toggle',
	K: 'toggle',
	MediaPlayPause: 'toggle',
	MediaPlay: 'play',
	MediaPause: 'pause',
	MediaStop: 'stop',
	j: 'rewind',
	J: 'rewind',
	l: 'forward',
	L: 'forward',
	MediaRewind: 'rewind_long',
	MediaFastForward: 'forward_long',
	MediaTrackPrevious: 'rewind_long',
	MediaTrackNext: 'forward_long',
	m: 'mute',
	M: 'mute',
	AudioVolumeMute: 'mute',
	f: 'fullscreen',
	F: 'fullscreen',
	c: 'subtitles',
	C: 'subtitles',
	Escape: 'back',
	GoBack: 'back',
	BrowserBack: 'back'
};

/** Remote key codes for keys whose `key` is "Unidentified" on TVs. */
const BY_CODE: Record<number, PlayerCommand> = {
	10252: 'toggle', // Tizen MediaPlayPause
	415: 'play',
	19: 'pause',
	413: 'stop',
	412: 'rewind_long',
	417: 'forward_long',
	10009: 'back', // Tizen Return
	461: 'back' // webOS Back
};

export function commandOf(
	event: Pick<KeyboardEvent, 'key' | 'keyCode' | 'altKey' | 'ctrlKey' | 'metaKey'>
): PlayerCommand | null {
	if (event.altKey || event.ctrlKey || event.metaKey) return null;
	return BY_KEY[event.key] ?? BY_CODE[event.keyCode] ?? null;
}

/** Seconds a seek command moves by. */
export function seekStep(command: PlayerCommand): number {
	switch (command) {
		case 'rewind':
			return -10;
		case 'forward':
			return 10;
		case 'rewind_long':
			return -30;
		case 'forward_long':
			return 30;
		default:
			return 0;
	}
}
