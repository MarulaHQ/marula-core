# core-db

> **Purpose:** SQLx compile-time-checked queries and migrations. Every user-data table has org_id + an RLS policy (CI-enforced).
> **Non-purpose:** No business logic; no queries outside migration transactions in other crates.

Schema changes follow the expand/contract rules and `--breaking` flagging convention in [`docs/upgrade-compatibility-policy.md`](../../docs/upgrade-compatibility-policy.md) §4 (mechanically enforced by `scripts/check-migrations.sh`).

