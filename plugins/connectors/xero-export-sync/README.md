# xero-export-sync

Purpose: first-party HTTP adapter that pushes Marula invoices, payments and
credit notes into a customer-provided Xero tenant over the PUBLIC Xero
accounting API, and reads that tenant's chart of accounts and tax rates back —
the Stage-2 acceptance pattern proven by the NinjaOne and Hudu reference
connectors, in Rust crate form (`ExportSink` trait + reference implementation,
Apache-2.0).

Status: implemented (Wave 9 lane `p9-adapters-export`, first tranche; credit
notes, the read leg and reconcile added for claims-gaps G1–G3).

## What it does

- `ExportSink::push_invoice` maps a Marula invoice to
  `POST /api.xro/2.0/Invoices` (`Type: ACCREC`, `Contact.ContactID`,
  `LineItems`, `InvoiceNumber`, `CurrencyCode`) and carries
  `[marula:invoice:{id}]` in `Reference` so the vendor-side record
  stays auditable back to its source. Each line carries `TaxType` when the
  caller resolves a vendor tax rate; the adapter never substitutes a default.
- `ExportSink::push_payment` maps a payment to
  `POST /api.xro/2.0/Payments`, linking to the previously exported
  invoice via `Invoice.InvoiceID` (= `ExportReceipt.external_id` from
  the invoice push) and applying it through `Account.Code`.
- `ExportSink::push_credit_note` maps a credit to
  `POST /api.xro/2.0/CreditNotes` as `ACCRECCREDIT` and allocates it to the
  invoice it credits through `Allocations[].Invoice.InvoiceID` — the parent
  invoice's vendor id from the stored receipt — with `Allocations[].Amount` set
  to the priced line total. It carries `[marula:credit_note:{id}]` in
  `Reference`. An allocation is what reduces the invoice; a note without one is
  credit the customer's books show twice, so a missing parent id is an
  `InvalidRequest` before any request is built.
- `ExportSink::read_accounts` / `read_tax_codes` are the read leg
  (`GET /Accounts`, `GET /TaxRates`). They answer in a NEUTRAL shape:
  `LedgerAccount` carries an `AccountKind` bucket (`bank`, `income`,
  `long_term_liability`, …, `unknown`), never Xero's `CURRLIAB`/`OVERHEADS`
  strings, and `TaxCode` carries the `TaxType` a line references plus a
  basis-point rate from `EffectiveRate`.
- `ExportSink::find_document` is the reconcile: it filters the entity on the
  document number (`where=InvoiceNumber=="…"` / `CreditNoteNumber=="…"`) and
  accepts a row only when its `Reference` carries
  `[marula:<kind>:<uuid>]`. The marker is what proves a vendor record is ours.
- Requests authenticate with a bearer token plus the mandatory
  `Xero-tenant-id` header; responses are parsed from Xero's
  array-wrapped envelope into a vendor-neutral `ExportReceipt`.
- Errors are typed and vendor-free: `Disabled`, `InvalidRequest`,
  `Unauthorized`, `RateLimited`, `Vendor { status }`, `Network`.
  `ExportError::retryable()` encodes the only split a caller needs: a 4xx is the
  request's fault, a 429/5xx/transport failure is "not now", and a credential
  is a human's job.

## Transport

Every call has a bounded budget: `XeroConfig::timeout` (default 30s, the whole
request) plus a fixed 10s connect timeout. A constructor that cannot build the
client is fallible rather than handing back a client with no timeout. Transport
errors keep the reason and drop the URL (`reqwest::Error::without_url`), and
`XeroConfig`'s `Debug` prints `<redacted>` for the token.

## Credentials (BYOK, sealed)

Xero uses OAuth2 with the `accounting.transactions` scope. The operator
supplies their own access token + tenant id; Marula hosts no OAuth app
and performs no token refresh. Core seals the credential document
(`{"access_token": "…", "tenant_id": "…"}`) in
`connector_connections.secrets_encrypted` and unseals it just-in-time; a
non-secret `base_url` may ride the connection's `settings` for a sandbox host.
When a token expires, the operator re-seals a fresh one through the connection
surface.

## None mode (standalone-first doctrine)

The shipped default is `NoneExportSink`: zero network, zero vendor
dependencies, and a typed `ExportError::Disabled` on every call. An org
with no accounting credentials runs fully standalone — call sites treat
`Disabled` as "would have synced". No HTTP client is even constructed in
none mode. `find_document` answers `Disabled` rather than `Ok(None)`: a caller
must never read "nothing found" as permission to push.

## Not done here (deliberate)

- No vendor SDK — the Xero accounting API is called over plain HTTP
  (`api.xro/2.0`).
- No OAuth dance, token refresh, or hosted app (see connectors/README).
- No background sync scheduling — the polling/sync worker job is a
  documented future worker job, not part of this adapter.
- No contact/Account catalog management: the caller must pass an
  existing Xero `ContactID` and an `Account.Code`; the adapter never
  invents vendor records.
- No void or refund, and no `LineItem.AccountCode`: the export's line items do
  not yet name an income account, which a Xero ACCREC invoice wants when the
  line has no item behind it (see below).

## Tests

`cargo test -p xero-export-sync` runs contract tests against a local
axum stub server, pinning the wire shape (paths, bearer auth, tenant
header, ACCREC/ACCRECCREDIT types, line amounts in minor-unit decimals,
`Reference` provenance, the credit note's allocation, array-wrapped
responses), the read leg's neutral shape, the reconcile's marker check, the
typed error mapping (401/429/400) with its retryability split, the timeout, and
the none-mode guarantees.

## What a fixture does not prove

These are sandbox-confirm items, not claims:

- **That an inline `Allocations` array on `POST /CreditNotes` is applied.**
  Xero documents allocations (and a separate `PUT /CreditNotes/{id}/Allocations`
  route); whether the create call honours them in one step has to be seen in a
  tenant. If it does not, the adapter's create must be followed by the
  allocations call, and that changes the retry story — which is why it is named
  here rather than assumed.
- **`LineItem.AccountCode`.** A Xero ACCREC line normally names an income
  account; this adapter does not send one yet. Bank-connected tenants may accept
  it, others may refuse with a validation error.
- **`where` filtering.** The reconcile relies on `InvoiceNumber` /
  `CreditNoteNumber` equality filters; behaviour at a tenant's limits and for a
  document number Xero rewrote was not observed.
- **Token expiry.** A rotated token surfaces as `Unauthorized` through the 401
  mapping; a real 401 (Xero returns 401 with an empty body for an expired token)
  was not exercised.
