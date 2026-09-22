# How to manage API keys

> **Audience:** admin — machine credentials are settings-owner work.

## Prerequisites

- Admin access to **Settings→API keys**.
- Knowledge of what the integration will actually call, because scopes
  are per-resource and per-verb. The pain of revoking later outweighs
  the two minutes of scoping now.

## The lifetime, in the surface's own words

> "Bearer credentials for machine integrations. Secrets are shown once at
> creation; to rotate, create a replacement key then revoke this one."

So the lifecycle is strictly **show once → rotate by replacement →
revoke** — there is no re-display and no edit-in-place anywhere in the
product.

## 1. Create the key with the narrowest scopes

The scope picker lists the platform's resource:verb pairs
(`tickets:read/write`, `clients:read/write`, `timers:read/write`) and
carries its own instruction: "pick the narrowest set the integration
needs". Two keys with different scopes beat one key with wide scopes for
anything that must not write.

## 2. Store the secret once

The full secret is displayed exactly once at creation. The table keeps
only the **prefix** (`mrp_dem…`) plus scopes and last-used, safe to
screenshot and useless to an attacker. Copy the secret into the
integration's secret store at this point. If it lands anywhere logs can
reach, treat it as leaked and rotate immediately.

## 3. Rotate by replacement, not by edit

When a rotation is due: create a **replacement** key with the same or
adjusted scopes, deploy it to the integration, verify it authenticates,
then revoke the old key. Deleting before the replacement is live is how
integrations go dark — the ordering is deliberate in the UI copy.

## 4. Revoke on suspicion or retirement

Every row has **Revoke**. Revoke on: staff-offboarding touching the
integration, a leaked secret, an integration being unplugged, or rotation
completion. "Last used" is on the row, so an unused key_candidate for
revocation is trivially evidenced.

## 5. Audit posture

Key creation and revocation land in the
[audit trail](../explanation/provenance-vs-audit.md) like every other
administrative event. If your SOP requires periodic reviews, the API keys
table itself is the review surface: prefix, scopes, last used, one row
each, no proliferation.

## Related machine integration surfaces

- Signed **webhooks** (HMAC-signed, retried with backoff, every attempt in
  the delivery log) live under Settings→Integrations with their own
  Disable / Send test / Rotate / Delivery log / Delete actions — see
  [automation rules](admin-automation-rules.md) for what can trigger them.
- Connectors and plugin capabilities are separate grants. See
  [plugin consent](admin-plugin-consent.md).
