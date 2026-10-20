/**
 * Subtitle cues drawn by the player itself instead of the browser: they
 * can move above the controls, scale with the screen and keep one style
 * on every TV. Cue text is WebVTT markup; only <i>, <b> and <u> survive,
 * everything else (voices, classes, ruby, timestamps) is dropped. The
 * result is plain data rendered as text, never as HTML.
 */

export interface CueSegment {
	text: string;
	italic: boolean;
	bold: boolean;
	underline: boolean;
}

const ENTITIES: Record<string, string> = {
	'&amp;': '&',
	'&lt;': '<',
	'&gt;': '>',
	'&nbsp;': '\u00a0',
	'&lrm;': '\u200e',
	'&rlm;': '\u200f',
	'&quot;': '"',
	'&#39;': "'"
};

function decode(text: string): string {
	return text.replace(
		/&(?:amp|lt|gt|nbsp|lrm|rlm|quot|#39);/g,
		(entity) => ENTITIES[entity] ?? entity
	);
}

/** One cue's text as lines of styled segments. */
export function parseCue(text: string): CueSegment[][] {
	const style = { italic: 0, bold: 0, underline: 0 };
	return text.split(/\r?\n/).map((line) => {
		const segments: CueSegment[] = [];
		for (const part of line.split(/(<\/?[^>]*>)/)) {
			if (!part) continue;
			const tag = /^<(\/?)([a-z]+)/i.exec(part);
			if (tag || part.startsWith('<')) {
				const closing = tag?.[1] === '/';
				const name = tag?.[2]?.toLowerCase();
				const key =
					name === 'i' ? 'italic' : name === 'b' ? 'bold' : name === 'u' ? 'underline' : null;
				if (key) style[key] = Math.max(0, style[key] + (closing ? -1 : 1));
				continue;
			}
			const value = decode(part);
			const last = segments.at(-1);
			const current = {
				italic: style.italic > 0,
				bold: style.bold > 0,
				underline: style.underline > 0
			};
			if (
				last &&
				last.italic === current.italic &&
				last.bold === current.bold &&
				last.underline === current.underline
			) {
				last.text += value;
			} else {
				segments.push({ text: value, ...current });
			}
		}
		return segments;
	});
}

/** Text tracks that carry subtitles (not hls.js metadata or chapters). */
export function isSubtitleTrack(track: TextTrack): boolean {
	return track.kind === 'subtitles' || track.kind === 'captions';
}

/** The lines on screen now: active cues of every subtitle track in use. */
export function activeLines(tracks: TextTrackList | TextTrack[]): CueSegment[][] {
	const lines: CueSegment[][] = [];
	for (const track of Array.from(tracks)) {
		if (!isSubtitleTrack(track) || track.mode === 'disabled' || !track.activeCues) continue;
		for (const cue of Array.from(track.activeCues)) {
			const text = (cue as TextTrackCue & { text?: string }).text;
			if (text) lines.push(...parseCue(text).filter((line) => line.some((s) => s.text.trim())));
		}
	}
	return lines;
}
