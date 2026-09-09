CREATE TABLE webhook_endpoints (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    destination_type TEXT NOT NULL CHECK(destination_type IN ('generic_json','discord','slack')),
    subscribed_events TEXT[] NOT NULL DEFAULT ARRAY['item_added'],
    enabled BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE webhook_deliveries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    endpoint_id UUID NOT NULL REFERENCES webhook_endpoints(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','processing','delivered','failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_attempt_at TIMESTAMPTZ,
    response_code INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX webhook_pending ON webhook_deliveries(next_attempt_at) WHERE status IN ('pending','processing');
CREATE TABLE webhook_delivery_attempts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    delivery_id UUID NOT NULL REFERENCES webhook_deliveries(id) ON DELETE CASCADE,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    response_code INTEGER,
    error_code TEXT
);
CREATE FUNCTION enqueue_media_webhooks() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO webhook_deliveries(endpoint_id,event_type,payload)
    SELECT id,'item_added',jsonb_build_object('event_type','item_added','item_id',NEW.id,'title',NEW.title)
    FROM webhook_endpoints WHERE enabled AND 'item_added'=ANY(subscribed_events);
    RETURN NEW;
END;
$$;
CREATE TRIGGER media_webhooks AFTER INSERT ON media_items FOR EACH ROW EXECUTE FUNCTION enqueue_media_webhooks();
