# xero-export-sync

Purpose: first-party HTTP adapter that pushes Meridian invoices and
payments into a customer-provided Xero tenant over the PUBLIC Xero
accounting API — the Stage-2 acceptance pattern proven by the NinjaOne
and Hudu reference connectors, in Rust crate form (`ExportSink` trait +
reference implementation, Apache-2.0).

Status: implemented (Wave 9 lane `p9-adapters-export`, first tranche).

## What it does

- `ExportSink::push_invoice` maps a Meridian invoice to
  `POST /api.xro/2.0/Invoices` (`Type: ACCREC`, `Contact.ContactID`,
  `LineItems`, `InvoiceNumber`, `CurrencyCode`) and carries
  `[meridian:invoice:{id}]` in `Reference` so the vendor-side record
  stays auditable back to its source.
- `ExportSink::push_payment` maps a payment to
  `POST /api.xro/2.0/Payments`, linking to the previously exported
  invoice via `Invoice.InvoiceID` (= `ExportReceipt.external_id` from
  the invoice push) and applying it through `Account.Code`.
- Requests authenticate with a bearer token plus the mandatory
  `Xero-tenant-id` header; responses are parsed from Xero's
  array-wrapped envelope into a vendor-neutral `ExportReceipt`.
- Errors are typed and vendor-free: `Disabled`, `InvalidRequest`,
  `Unauthorized`, `RateLimited`, `Vendor { status }`, `Network`.

## Credentials (BYOK, sealed)

Xero uses OAuth2 with the `accounting.transactions` scope. The operator
supplies their own access token + tenant id; Meridian hosts no OAuth app
and performs no token refresh. Tokens ride the core's generic BYOK
sealed store (`byok_secrets`, provider id `xero`): AES-256-GCM at rest,
admin-gated writes, never logged, never returned by any API. When a
token expires, the operator re-seals a fresh one through the same admin
surface.

## None mode (standalone-first doctrine)

The shipped default is `NoneExportSink`: zero network, zero vendor
dependencies, and a typed `ExportError::Disabled` on every call. An org
with no accounting credentials runs fully standalone — call sites treat
`Disabled` as "would have synced". No HTTP client is even constructed in
none mode.

## Not done here (deliberate)

- No vendor SDK — the Xero accounting API is called over plain HTTP
  (`api.xro/2.0`).
- No OAuth dance, token refresh, or hosted app (see connectors/README).
- No background sync scheduling — the polling/sync worker job is a
  documented future worker job, not part of this adapter.
- No contact/Account catalog management: the caller must pass an
  existing Xero `ContactID` and an `Account.Code`; the adapter never
  invents vendor records beyond the invoice/payment pair.

## Tests

`cargo test -p xero-export-sync` runs contract tests against a local
axum stub server, pinning the wire shape (paths, bearer auth, tenant
header, ACCREC type, line amounts in minor-unit decimals, Reference
provenance, array-wrapped responses), the typed error mapping
(401/429/400), and the none-mode guarantees.
