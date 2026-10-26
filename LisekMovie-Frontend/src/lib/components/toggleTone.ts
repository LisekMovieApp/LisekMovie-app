/**
 * Colours of the round state buttons (Faza 12): watched (✓), favourite (♥)
 * and the gold Play next to them.
 *
 * Before, a button's class was "base + `text-neutral-100`" with
 * `text-gold` appended when active. Both utilities then sat on the element
 * and the one later in Tailwind's stylesheet won — `text-neutral-100`, so
 * a watched title kept a white check. Here the shape (size, rounding,
 * focus outline) and the colours are separate: a button gets exactly one
 * colour set, chosen by state, never two competing ones.
 *
 * - Idle: light icon, gold on hover and remote focus.
 * - Watched: a solid gold disc with a black check — readable from the
 *   sofa, and unlike a recoloured icon it does not depend on hue alone.
 * - Favourite: a red heart.
 */

export type ToggleKind = 'watched' | 'favorite';
/** `plain` on the page background, `subtle` for dense lists (dimmer idle
 * icon), `overlay` on artwork (dark disc behind the icon). */
export type ToggleSurface = 'plain' | 'subtle' | 'overlay';

/** Shape and focus ring shared by every state button; add a `size-*`. */
export const TOGGLE_SHAPE =
	'grid shrink-0 place-items-center rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-gold disabled:opacity-50';

const IDLE: Record<ToggleSurface, string> = {
	plain:
		'text-neutral-100 hover:bg-white/10 hover:text-gold focus-visible:bg-white/15 focus-visible:text-gold',
	subtle:
		'text-neutral-300 hover:bg-white/10 hover:text-gold focus-visible:bg-white/15 focus-visible:text-gold',
	overlay:
		'bg-black/60 text-neutral-100 backdrop-blur hover:bg-black/80 hover:text-gold focus-visible:bg-black/90 focus-visible:text-gold'
};

const WATCHED =
	'bg-gold text-black shadow-[0_0_0_1px_rgb(0_0_0/0.35)] hover:bg-[#d6bb66] focus-visible:bg-[#d6bb66]';

const FAVORITE: Record<ToggleSurface, string> = {
	plain:
		'text-red-500 hover:bg-white/10 hover:text-red-400 focus-visible:bg-white/15 focus-visible:text-red-400',
	subtle:
		'text-red-500 hover:bg-white/10 hover:text-red-400 focus-visible:bg-white/15 focus-visible:text-red-400',
	overlay:
		'bg-black/60 text-red-500 backdrop-blur hover:bg-black/80 hover:text-red-400 focus-visible:bg-black/90 focus-visible:text-red-400'
};

/** The gold Play button beside the toggles. */
export const PLAY_TONE = 'bg-gold text-black hover:bg-[#d6bb66] focus-visible:bg-[#d6bb66]';

/** The one colour set for a state button. */
export function toggleTone(
	kind: ToggleKind,
	active: boolean,
	surface: ToggleSurface = 'plain'
): string {
	if (!active) return IDLE[surface];
	return kind === 'watched' ? WATCHED : FAVORITE[surface];
}
