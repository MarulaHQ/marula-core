# qbo-export-sync

Purpose: first-party HTTP adapter that pushes Marula invoices and
payments into a customer-provided QuickBooks Online company over the
PUBLIC QBO REST API — the Stage-2 acceptance pattern proven by the
NinjaOne and Hudu reference connectors, in Rust crate form (`ExportSink`
trait + reference implementation, Apache-2.0).

Status: implemented (Wave 9 lane `p9-adapters-export`, first tranche).

## What it does

- `ExportSink::push_invoice` maps a Marula invoice to
  `POST /v3/company/{realmId}/invoice` (ACCREC-style `CustomerRef`,
  `Line` with `SalesItemLineDetail`, `DocNumber`, `CurrencyRef`), and
  carries `[marula:invoice:{id}]` in `PrivateNote` so the vendor-side
  record stays auditable back to its source.
- `ExportSink::push_payment` maps a payment to
  `POST /v3/company/{realmId}/payment`, linking to the previously
  exported invoice via `Line.LinkedTxn` (`TxnId` = the vendor invoice
  `Id` returned by `ExportReceipt.external_id`).
- Responses are parsed into a vendor-neutral `ExportReceipt`
  (`external_id` + `external_number`), which the caller stores to make
  re-pushes idempotent and to attach payments.
- Errors are typed and vendor-free: `Disabled`, `InvalidRequest`,
  `Unauthorized`, `RateLimited`, `Vendor { status }`, `Network`.

## Credentials (BYOK, sealed)

QBO uses OAuth2 with the `com.intuit.quickbooks.accounting` scope. The
operator supplies their own access token + realm id; Marula hosts no
OAuth app and performs no token refresh. Tokens ride the core's generic
BYOK sealed store (`byok_secrets`, provider id `qbo`): AES-256-GCM at
rest, admin-gated writes, never logged, never returned by any API. When
a token expires (QBO tokens rotate), the operator re-seals a fresh one
through the same admin surface.

## None mode (standalone-first doctrine)

The shipped default is `NoneExportSink`: zero network, zero vendor
dependencies, and a typed `ExportError::Disabled` on every call. An org
with no accounting credentials runs fully standalone — call sites treat
`Disabled` as "would have synced". No HTTP client is even constructed in
none mode.

## Not done here (deliberate)

- No vendor SDK — the QBO REST surface is called over plain HTTP with
  pinned `minorversion=75`.
- No OAuth dance, token refresh, or hosted app (see connectors/README).
- No background sync scheduling — the polling/sync worker job is a
  documented future worker job, not part of this adapter.
- No customer/item catalog management: the caller must pass an existing
  QBO customer id (`CustomerRef.value`); the adapter never invents
  vendor records beyond the invoice/payment pair.

## Tests

`cargo test -p qbo-export-sync` runs contract tests against a local
axum stub server, pinning the wire shape (path, bearer auth, line
amounts in minor-unit decimals, PrivateNote provenance), the typed
error mapping (401/429/400), and the none-mode guarantees.
