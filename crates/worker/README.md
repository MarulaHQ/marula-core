# worker

> **Purpose:** Postgres-backed async job queue and all job definitions: email send/receive, SLA evaluation, webhook delivery, automations, reports, AI calls, plugin invocations, calendar sync.
> **Non-purpose:** Does not write the database directly — enqueues effects through public APIs or dedicated job handlers reviewed like API endpoints.
