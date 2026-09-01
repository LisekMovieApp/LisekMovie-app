-- Faza 5 (round 2): self-hosters expect a choice in where downloaded
-- metadata images live — writing directly
-- alongside the media itself rather than only in an app-private cache.

-- Per-library switch: when true (the default — most self-hosters expect
-- this and it keeps the library portable to another tool later), a
-- provider match's poster/backdrop are written directly next to the video
-- file (poster.jpg / backdrop.jpg). When false, they always go straight to
-- the local config-dir cache instead.
ALTER TABLE libraries ADD COLUMN save_metadata_in_media_folder BOOLEAN NOT NULL DEFAULT true;

-- Set when a media-folder image write was attempted (the library has the
-- switch above turned on) but failed — almost always a permissions problem
-- (a read-only network share, a mount the app's process can't write to) —
-- and the app fell back to the local config-dir cache instead. Surfaced
-- later by an Admin Panel warning view with a "move to the media folder"
-- action once the admin fixes the underlying permission issue; for now
-- this is only tracked, not yet actionable from the UI. Never set when the
-- switch above is simply turned off — that is expected behavior, not an
-- exception worth flagging.
ALTER TABLE media_items ADD COLUMN metadata_fallback BOOLEAN NOT NULL DEFAULT false;
