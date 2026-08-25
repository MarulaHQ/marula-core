# core-api

> **Purpose:** The Axum HTTP API — the only process that writes the database. Every mutation emits a domain event via the transactional outbox.
> **Non-purpose:** No business rules that belong in core-domain; no direct access from other services; no reads bypassing RLS.
