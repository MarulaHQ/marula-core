# core-events

> **Purpose:** Transactional outbox relay, versioned event envelope ({id,type,org_id,entity,payload_schema_ref,payload}), Redis stream publisher. The integration contract for webhooks, plugins, realtime, and every external consumer. Also owns the machine-checked event-evolution contract (`contract` module): the published-payload registry, golden fixtures (`fixtures/events/v1/`), the retired-type co-emit rail, and the deprecation-clock gate (U4).
> **Non-purpose:** No consumers live here — this crate publishes, it does not subscribe.
