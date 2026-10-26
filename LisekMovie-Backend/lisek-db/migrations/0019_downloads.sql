-- Downloads (Faza 7): an account may download original files unless an
-- administrator turns it off. Existing rows keep downloads allowed.
ALTER TABLE user_restrictions
    ADD COLUMN can_download BOOLEAN NOT NULL DEFAULT true;
