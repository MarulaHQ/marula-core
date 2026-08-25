# core-domain

> **Purpose:** Pure domain layer: entities, semantic status categories, policy module (all permission checks), domain services. Heavily tested, zero I/O.
> **Non-purpose:** No SQL, no HTTP, no clocks — time is injected. If it imports tokio or sqlx, it does not belong here.
