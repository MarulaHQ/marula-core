# qbo-export-sync

Purpose: first-party HTTP adapter that pushes Marula invoices, payments and
credit notes into a customer-provided QuickBooks Online company over the PUBLIC
QBO REST API, and reads that company's chart of accounts and tax codes back —
the Stage-2 acceptance pattern proven by the NinjaOne and Hudu reference
connectors, in Rust crate form (`ExportSink` trait + reference implementation,
Apache-2.0).

Status: implemented (Wave 9 lane `p9-adapters-export`, first tranche; credit
notes, the read leg and reconcile added for claims-gaps G1–G3).

## What it does

- `ExportSink::push_invoice` maps a Marula invoice to
  `POST /v3/company/{realmId}/invoice` (ACCREC-style `CustomerRef`,
  `Line` with `SalesItemLineDetail`, `DocNumber`, `CurrencyRef`), and
  carries `[marula:invoice:{id}]` in `PrivateNote` so the vendor-side
  record stays auditable back to its source. Each line carries
  `SalesItemLineDetail.TaxCodeRef` when the caller resolves a vendor tax code;
  the adapter never substitutes a default.
- `ExportSink::push_payment` maps a payment to
  `POST /v3/company/{realmId}/payment`, linking to the previously
  exported invoice via `Line.LinkedTxn` (`TxnId` = the vendor invoice
  `Id` returned by `ExportReceipt.external_id`).
- `ExportSink::push_credit_note` maps a credit to
  `POST /v3/company/{realmId}/creditmemo` and links it to the invoice it
  adjusts through `LinkedTxn` (`TxnType: "Invoice"`), using the parent
  invoice's vendor `Id` from the stored receipt. It stamps
  `[marula:credit_note:{id}]` into `PrivateNote`. An unlinked credit memo is a
  second document for the same money, so a missing parent id is an
  `InvalidRequest` before any request is built.
- `ExportSink::read_accounts` / `read_tax_codes` are the read leg. They answer
  `GET /query` (`select * from Account`, `TaxCode`, `TaxRate`, paged with
  `STARTPOSITION`/`MAXRESULTS`) in a NEUTRAL shape: `LedgerAccount` carries an
  `AccountKind` bucket (`bank`, `income`, `cost_of_goods_sold`, …, `unknown`),
  not QBO's `AccountType` string, and `TaxCode` carries the `TaxCode.Id` a line
  references plus a basis-point rate resolved through the `TaxRate` entities the
  code points at.
- `ExportSink::find_document` is the reconcile: it queries the entity by
  `DocNumber` and accepts a row only when its `PrivateNote` carries
  `[marula:<kind>:<uuid>]`. A document number is not unique in QBO, so the
  marker is what proves a vendor record is ours.
- Responses are parsed into a vendor-neutral `ExportReceipt`
  (`external_id` + `external_number`), which the caller stores to make
  re-pushes idempotent and to attach payments and credit notes.
- Errors are typed and vendor-free: `Disabled`, `InvalidRequest`,
  `Unauthorized`, `RateLimited`, `Vendor { status }`, `Network`.
  `ExportError::retryable()` encodes the only split a caller needs: a 4xx is the
  request's fault (retrying re-sends the refusal), a 429/5xx/transport failure
  is "not now", and a credential is a human's job.

## Transport

Every call has a bounded budget: `QboConfig::timeout` (default 30s, the whole
request) plus a fixed 10s connect timeout. A constructor that cannot build the
client is fallible rather than handing back a client with no timeout. Transport
errors keep the reason and drop the URL (`reqwest::Error::without_url`),
because a sandbox base URL is often written with credentials in it, and
`QboConfig`'s `Debug` prints `<redacted>` for the token.

## Credentials (BYOK, sealed)

QBO uses OAuth2 with the `com.intuit.quickbooks.accounting` scope. The
operator supplies their own access token + realm id; Marula hosts no
OAuth app and performs no token refresh. Core seals the credential document
(`{"access_token": "…", "realm_id": "…"}`) in
`connector_connections.secrets_encrypted` and unseals it just-in-time; a
non-secret `base_url` may ride the connection's `settings` for a sandbox host.
When a token expires the operator re-seals a fresh one through the connection
surface.

## None mode (standalone-first doctrine)

The shipped default is `NoneExportSink`: zero network, zero vendor
dependencies, and a typed `ExportError::Disabled` on every call. An org
with no accounting credentials runs fully standalone — call sites treat
`Disabled` as "would have synced". No HTTP client is even constructed in
none mode. `find_document` answers `Disabled` rather than `Ok(None)`: a caller
must never read "nothing found" as permission to push.

## Not done here (deliberate)

- No vendor SDK — the QBO REST surface is called over plain HTTP with
  pinned `minorversion=75`.
- No OAuth dance, token refresh, or hosted app (see connectors/README).
- No background sync scheduling — the polling/sync worker job is a
  documented future worker job, not part of this adapter.
- No customer/item catalog management: the caller must pass an existing
  QBO customer id (`CustomerRef.value`); the adapter never invents
  vendor records.
- No void or refund; no `open_url`, because QBO has no supported transaction
  deep link (`txnId` is not the API `Id`, and a constructed link can open
  another company's invoice — `docs/reference/connector-capabilities.md` §3.4).

## Tests

`cargo test -p qbo-export-sync` runs contract tests against a local
axum stub server, pinning the wire shape (path, bearer auth, line
amounts in minor-unit decimals, `PrivateNote` provenance, `TaxCodeRef`,
the credit memo's `LinkedTxn`), the read leg's neutral shape (including an
account type nobody planned landing in `Unknown`), the reconcile's marker
check, the typed error mapping (401/429/400/502) with its retryability split,
the timeout, and the none-mode guarantees.

## What a fixture does not prove

These are sandbox-confirm items, not claims:

- **That a `LinkedTxn` on a credit memo at creation applies the credit.**
  Intuit documents the link; whether the invoice balance moves on create (rather
  than needing a separate `Payment` against the credit memo) has to be seen in a
  company. The fixture proves what we send, not what QBO does with it.
- **Paging.** The read leg walks `STARTPOSITION` pages and stops on a short page;
  that a real company reports rows the way the stub does is unverified, and the
  walk is capped at 5 pages of 1000.
- **A real validation error.** The typed split is asserted per status; the field
  names QBO puts in a 400 body were not observed against a live company.
- **Token expiry.** A rotated or expired token surfaces as `Unauthorized`
  through the 401 mapping; a real 401 and the re-seal flow that fixes it were
  not exercised.
