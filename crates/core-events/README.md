# core-events

> **Purpose:** Transactional outbox relay, versioned event envelope ({id,type,org_id,entity,payload_schema_ref,payload}), Redis stream publisher. The integration contract for webhooks, plugins, realtime, and every external consumer.
> **Non-purpose:** No consumers live here — this crate publishes, it does not subscribe.
