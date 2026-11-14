# Connector capabilities: what a connector can reach

> **Purpose:** the catalogue a connector author reads before declaring
> anything, and the form they fill in when the catalogue does not cover
> them. It exists so a vendor integration is a declaration rather than a
> conversation about layout.
> **Companions:** [ADR 0032](../adr/0032-connector-connection-model.md),
> [ADR 0033](../adr/0033-record-links-and-external-references.md),
> [connector-surface.md](connector-surface.md).
> **Status:** proposed, 2026-11-14.

## 1. Three vocabularies, and only one of them is open

The design holds because the three lists change at different rates and for
different reasons.

| Vocabulary | Who owns it | Closed? | Adding one costs |
|---|---|---|---|
| **Behaviour** — what an action *does* | Core | Closed, five | An RFC: the UI, the audit and the permission check all change |
| **Product kind** — what kind of vendor this is | The connector's manifest | Open, namespaced | Nothing. The UI renders it |
| **Record type** — which Marula record it hangs off | Core | Closed, registered | An RFC: core must be able to show that record |

A new product kind is data. A new behaviour is code. A new record type is a
product decision. Mixing those three into one "capability" string is what
turns a connector system into a pile of exceptions, so the boundary is
written down here and tested: the manifest's `product_kinds` is a free
string list, `behaviours` cannot be declared at all, and `record_type` is
validated against a registry with a typed refusal naming the allowed set.

## 2. The five behaviours

| Behaviour | What the press does | Secret spent | Audited | Needs |
|---|---|---|---|---|
| `open` | Builds a URL from the connection's base URL and the external id | no | as a view | a URL template |
| `launch` | Asks the adapter to start or join something and returns a one-time URL (or a protocol URI) | yes | as an action | an adapter method + a scope |
| `dial` | A `launch` whose target is a phone number, resolved to the caller's own extension | yes | as an action | a per-user external identity |
| `share` | Posts a Core-rendered card to a bound destination | no (binding secret) | as an action | a binding |
| `sync` | A background movement of records, never a user press | yes | as a job | a sink + identity keys |

`verify` is the canary, not a behaviour: it proves a connection works and is
never shown as a record action.

## 3. Product kinds and what they light up

The matrix below is the whole of what the staff UI does today. A connector
declaring a kind inherits its row. A connector declaring nothing gets
settings and the canary.

| Kind | Typical records | Behaviours | Notes |
|---|---|---|---|
| `rmm` | configuration | `open`, `launch` (remote shell/session where the vendor offers one) | One chip per RMM presence. The primary is the org default |
| `remote_access` | configuration | `launch` | The session is minted per press and audited. A static deep link is the degraded path, never the primary |
| `telephony` | contact, client, ticket (requester) | `dial`, `open` (call log) | Dials from the **caller's** extension via `user_external_identities`, never the record owner's |
| `accounting` | invoice, payment, credit_note, quote | `open`, `sync`, `read` | `open` lands on the vendor's document. `read` feeds accounts and tax codes back (the G2 leg). `sync` is the export |
| `dms` | kb_article, configuration | `open`, `sync` | `open` is the article in the vendor. Link validity is a sync outcome, not a press |
| `dns` | domain | `open`, `launch` (record write), `verify` | The write path stays behind the operator's explicit act |
| `identity` | user, contact | `open` | Read-mostly. Provisioning is `sync` |
| `network` | configuration | `open` | Read-only posture, per the UniFi boundary |
| `inventory` | configuration, purchase_order | `open`, `sync` | Stock movement is `sync`. Nothing writes back without the vendor's own workflow |
| `notifications` | — | `share` | Reaches S3 only. It has no record actions by design |
| `crm` | client, contact | `open`, `sync` | |

### Telephony in full

Click-to-call is not a ticket feature. It is a property of any record that
carries a phone number, and it fails in two specific ways when it is
implemented as a ticket button:

1. **It dials from the wrong place.** A call must originate from the
   pressing technician's extension. That is `user_external_identities`
   (`identity_kind = 'pbx_extension'`), and when the mapping is missing the
   action answers with the link step instead of dialling from a shared
   extension.
2. **It loses the context.** The call log the PBX later reports must land on
   the record that started it. That is a `launch` returning a call id, and a
   `sync` that attaches the completed call to the same record — which is
   why the ticket↔call association is a first-class column in the 3CX
   adapter today and not an afterthought.

The UI is one phone affordance beside a number, in three places: the client
record's numbers, the ticket's requester, and the contact list. Dial is the
chip body. The menu lists "Copy number" and "Open call log".

### Accounting in full

"Open this invoice in QuickBooks" is `accounting` + `open` on the *invoice*
record, resolved through the `external_refs` row the export wrote. The
export adapter already returns an `ExportReceipt` with the vendor id. ADR
0033 gives it somewhere to live.

**But the two vendors are not the same, and pretending they are would ship a
link that opens the wrong document.** Verified 2026-11-14:

- **Xero documents deep links.** `https://developer.xero.com/documentation/api-guides/deep-link-xero`
  specifies `https://go.xero.com/organisationlogin/default.aspx?shortcode={ORG_SHORTCODE}&redirecturl=/AccountsReceivable/View.aspx?InvoiceID={INVOICE_ID}`,
  where `InvoiceID` is the API's own `InvoiceID`. The connection settings
  carry the short code. This is `open`, backed by a documented contract.
- **QuickBooks Online does not.** Intuit states that a direct link to a
  transaction is not supported, that the URL's `txnId` is **not** the API
  entity `Id`, and that a constructed link can open the wrong invoice when
  the user is signed into a different company (Intuit help article
  "Provide QuickBooks Online page url for Transactions". The sample-app
  template `app.qbo.intuit.com/app/invoice?txnId=%s&companyId=%s` is only
  usable when a `txnId` was actually observed). So the QBO connector
  declares **no `open_url` for invoices** and the chip degrades to
  `Open QuickBooks` on the company plus `Copy invoice number`.
- **Xero's "online invoice URL" is not this.** That endpoint returns a
  customer-facing page for viewing and paying. Staff need the `go.xero.com`
  deep link above.

The rule this produces, and the reason `external_refs` carries an optional
`external_url`: **a template is allowed only where the vendor documents it.**
A connector that cannot document its link declares none, and the UI shows
the honest fallback instead of a URL that resolves to someone else's
invoice. `external_url` exists for the case where a connector *learns* a
canonical document URL from the vendor itself (a webhook payload, a report
row, an API field that returns a URL) — it is used verbatim and never
templated.

Read-back (accounts, tax codes, rates) is `read` and belongs to the same
kind, because a mapping without the vendor's list is a guess. An unmapped
tax code holds the invoice (G3). It never exports a default that someone
has to reconcile later.

### Verified deep-link evidence, and what it obliges

A template is a claim about someone else's URL. Keep the claim true:

| Vendor | Status (checked 2026-11-14) | What the connector may declare |
|---|---|---|
| Slack | Documented: `slack://channel?team={T}&id={C}`, `slack://user`, `slack://app`. HTTPS fallback `https://slack.com/app_redirect?channel={C}`. Message permalinks via `chat.getPermalink` | `open` for channels and messages, with `team_id` and `channel_id` stored on the binding — not names |
| Microsoft Teams | Documented: must be `https://teams.microsoft.com/l/…` (browser address-bar URLs are not deep links in new Teams). `msteams://` is optional and can fail without the desktop client | `open` only. Never prefix `msteams:` onto an `https:` URL |
| Xero | Documented deep link (short code + `InvoiceID`) | `open` for invoices |
| QuickBooks Online | **Not supported**: `txnId` ≠ API `Id`, and a constructed link can land on another company's invoice | no `open_url`. Degrade to the company home plus the document number |
| ConnectWise Control (ScreenConnect) | Community-documented rather than contractual: the Host URL with `/Join` appended starts a desktop session (`…/Host#Access/{group}//{sessionId}/Join`) | `launch` via a URL template (no secret spent) and `open` for the host page. Keep the group path URL-encoded |
| 3CX | `tel:{number}` for a plain handset dial. `POST /xapi/v1/Users/Pbx.MakeCall` for a CTI call from the tech's own extension | `launch` in both flavours: `url_template` for `tel:`, `method` for MakeCall |
| UniFi, NinjaOne, Hudu | **Not verified here.** Each may have a per-record UI path, but none is published as a contract | nothing until the connector author verifies and records the date and source in the manifest review |

Two obligations follow. A connector whose vendor is in the last row ships
`open` only if it can name its evidence in review. Otherwise it ships
`sync` and no chip, which is honest. And when a vendor changes its URL, the
break is visible as one manifest line, not as a hunt through a renderer.

## 4. What a connector declares

One block, one file. Two examples, deliberately different in shape:

```json
{
  "product_kinds": ["remote_access"],
  "surfaces": {
    "external_ref": {
      "record": "configuration",
      "external_kind": "machine",
      "open_url": "{base_url}/Host#Access/Machines/{external_id}",
      "launch": { "method": "remote_session", "label": "Start session",
                  "requires_scope": "sessions.start" }
    }
  }
}
```

```json
{
  "product_kinds": ["accounting"],
  "surfaces": {
    "external_ref": {
      "record": "invoice",
      "external_kind": "invoice",
      "open_url": "https://go.xero.com/organisationlogin/default.aspx?shortcode={settings.org_shortcode}&redirecturl=/AccountsReceivable/View.aspx?InvoiceID={external_id}"
    },
    "read": { "accounts": true, "tax_codes": true }
  }
}
```

The QuickBooks Online connector declares the same block **without**
`open_url`, because Intuit does not support transaction deep links (§3).

`{base_url}` and `{settings.*}` resolve from the connection, `{external_id}`
from the reference. A template that names an unresolved placeholder does not
render an action.

## 5. When the catalogue does not cover you

The three cases, and what each costs:

**A vendor that fits an existing kind.** Declare it. No conversation needed.

**A vendor that fits no existing kind** — a stock-control system, a
licence-compliance scanner, a CCTV NVR. Declare a new `product_kind`
(`inventory_control`, say), map its records onto the *nearest existing
record type* (`configuration` for a monitored box, `purchase_order` for an
order), and declare `open` / `sync`. The UI draws a chip with your label
and your behaviour, and nothing in Core changes. This is the intended path
for most "weird" cases, and it is why product kinds are open strings.

**A record type Core does not have** — a vehicle, a licence, a site
certificate. This needs an RFC. A record type is not a label: it needs a
list page, a detail page, permissions, an export mapping and a place in
search. A connector that invents one produces data the product cannot show,
which is why the API refuses with a typed error that names what it does
accept.

### The request form

Copy this into the RFC or the issue. It is short because the design has
done the work of narrowing the question:

```
Vendor / connector:
Record type:            (from the registry, or "new: <name>")
What the tech does:     one sentence, in their words
Vendor surface:         a deep link built from an id, or an API call that
                        returns a session / one-time URL
Needs a secret?         no / yes (which scope)
Writes?                 nothing / a Core record / the vendor object
Per-client?             one instance per org, or one per client
Audience:               all staff, or a named role
```

If the answer to "vendor surface" is a deep link, the whole integration is a
template and a `sync` that fills the ids. If it is a session, the answer to
"needs a secret" decides whether it is `launch` and whether it needs a scope
the consent screen can name.

## 6. What a connector gets without writing UI

Declaring `product_kinds` and one `external_ref` block produces, on the day
it installs: a settings page rendered from its schema, a canary test, a
capability chip on every linked record, a row in the ticket's linked-records
strip, an action on any recommendation that targets one of its records, and
per-client connections when it declares `per_client`. Declaring `share`
additionally produces the destination picker and the card. None of that
requires a component, a route, or a review of Core's layout.

## 7. Non-goals

- **A general scripting surface.** A connector does not run arbitrary code
  inside Core. `launch` is one adapter method with a typed result.
- **Vendor write-back as a record action.** Writes belong to `sync` or to a
  vendor-declared workflow action with an explicit scope, never to a chip.
- **A per-connector settings page or menu item.** Both exist only as
  renderers of declarations.
