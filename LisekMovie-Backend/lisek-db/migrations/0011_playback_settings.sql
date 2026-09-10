CREATE TABLE playback_settings (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    settings JSONB NOT NULL DEFAULT '{}'::jsonb
);
INSERT INTO playback_settings(id) VALUES (true);

CREATE TABLE browser_notification_events (
    id BIGSERIAL PRIMARY KEY,
    item_id UUID NOT NULL REFERENCES media_items(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX browser_notification_events_created ON browser_notification_events(created_at);
CREATE FUNCTION enqueue_browser_notification() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO browser_notification_events(item_id) VALUES (NEW.id);
    RETURN NEW;
END;
$$;
CREATE TRIGGER media_browser_notifications AFTER INSERT ON media_items FOR EACH ROW EXECUTE FUNCTION enqueue_browser_notification();
