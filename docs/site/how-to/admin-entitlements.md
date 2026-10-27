---
description: "Install the signed entitlement file your provider issues, read back the revision Marula holds, and understand each refusal."
---

# How to install an entitlement file

> **Audience:** admin — installing a file changes what your provider's records
> say you own, so this is an owner or admin action.

A provider that sells you a hosted plan can issue a **signed entitlement file**
that describes what your organisation bought. The file is a JSON document with a
revision number and a signature. Marula verifies the signature, stores the
document, and reports which revision it holds.

Marula does not read the document to decide what your screens may do. Nothing in
the product switches a feature on or off because of it. The file is a record that
keeps your provider's billing and your install in agreement, and the revision is
how the two sides know they agree.

## Before you start

You need two things:

1. The entitlement file, exactly as your provider sent it. Do not reformat it.
   the signature covers the bytes.
2. At least one trusted signing key on this install. The operator command
   `marula-core-cli plugin key add` manages that key list. An install with no
   trusted key refuses every file, which is deliberate: a signature nobody can
   check is not a check.

The same key list verifies plugin artifacts. A key you add so an install will
accept a plugin publisher's packages can also sign entitlement files, so add
keys from publishers you would trust with both.

## Install the file

Send the artifact and its signature to the org's entitlement endpoint. The
`artifact` object is the document your provider signed. `signature.sig` is the
ed25519 signature in hex.

```bash
curl -X PUT "https://your-host/api/v1/orgs/$ORG_ID/entitlements" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  --data-binary @entitlement-request.json
```

The request body has two members:

```json
{
  "artifact": {
    "schema": "marula.entitlement/v1",
    "org_id": "6f1c…",
    "revision": 7,
    "issued_at": "2026-10-27T00:00:00Z",
    "payload": { "seats": 25 },
    "payload_sha256": "…64 hex characters…"
  },
  "signature": { "key_id": "ops-1", "sig": "…128 hex characters…" }
}
```

The answer reports the outcome and the revision now held:

```json
{
  "org_id": "6f1c…",
  "outcome": "adopted",
  "entitlement": { "revision": 7, "payload": { "seats": 25 }, "…": "…" }
}
```

`outcome` is `adopted` when this call changed what Marula holds. It is
`unchanged` when you send the revision already installed with the same payload,
so a retry after a network error is safe to repeat.

## Check what is installed

```bash
curl "https://your-host/api/v1/orgs/$ORG_ID/entitlements" \
  -H "Authorization: Bearer $TOKEN"
```

`entitlement` is `null` until the org installs a file. There is no revision zero,
so "nothing installed" and "installed revision 0" are not the same state.

## When Marula refuses a file

Refusals happen before anything is stored. The response names a code, and the
document you sent is unchanged on disk so you can fix and resend it.

| Code | What it means | What to do |
|---|---|---|
| `bad_signature` | No trusted key verifies the signature | Check the file arrived unmodified, and that the issuer's key is installed |
| `no_trusted_keys` | This install trusts no signing key at all | Add the issuer's key with the operator command, then resend |
| `org_mismatch` | The file names a different org | Install it in the org it names. A valid signature for another tenant is still refused |
| `payload_digest_mismatch` | The payload does not match the digest in the file | Ask the provider to issue the file again. Something changed it in transit |
| `unsupported_schema` | The file is a format this release does not read | Upgrade Marula, or ask the provider for the current format |
| `invalid_revision` | The revision is missing, not a whole number, or not positive | Ask the provider to issue the file again |
| `artifact_too_large` | The document is over 64 KiB | Ask the provider to issue a smaller file. A real bundle is a few KiB |

A **stale revision** answers `409` instead of a code: the org already holds the
same or a higher revision with different content. Marula keeps what it has,
because a lower revision is older news and an equal revision with new content
means the issuer contradicted itself. Ask the provider which revision they
issued last.

## Where the record lives

The installed document is one row per org, in the `org_entitlements` table. Each
adoption writes an audit row and emits `org.entitlement_adopted` on the event
stream, carrying the revision and the payload digest but not the payload itself.
The event is how a subscriber learns the record moved without receiving a copy of
your commercial terms.
