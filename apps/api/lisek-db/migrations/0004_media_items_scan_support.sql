-- Faza 5: lets a library scan be re-run safely (ON CONFLICT upsert instead
-- of piling up duplicate rows every time), and lets the admin see and
-- control which libraries exist without touching the database by hand.

-- One media item per (library, file) — re-scanning a library must update
-- the existing row for a file it's already seen, not insert a second one.
ALTER TABLE media_items ADD CONSTRAINT media_items_library_file_unique UNIQUE (library_id, file_path);

-- Faza 1 defined `sort_order` as NOT NULL with no default, which is fine
-- for a manually-seeded row but not for an admin-facing "create library"
-- endpoint that doesn't ask the admin to pick a sort position up front —
-- new libraries land at the end of the list by default and can be
-- reordered later the same way provider priority already is.
ALTER TABLE libraries ALTER COLUMN sort_order SET DEFAULT 0;
