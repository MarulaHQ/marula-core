# realtime

> **Purpose:** WebSocket hub: subscribes to the event stream, per-user/per-board channels, presence, replay cursors. Read-only over Postgres.
> **Non-purpose:** Never writes the database; never owns business state — clients reconcile via REST snapshots.
