# threecx

Purpose: first-party 3CX connector written against ONLY the public APIs
(Apache-2.0) — the 3CX V20 Call Control / XAPI on the PBX side, the
Meridian VoIP ingest seam on the Meridian side. Reference implementation
of the agentic call-control adapter pattern (Wave 22 lane 3
`p22-ecosystem-connectors-gap`, leg B).

Status: LANDED — rides the Wave 22 lane 1 `p22-ecosystem-stretch` VoIP
connector API (`POST /api/v1/connectors/voip/call-logs` over an installed
`voip` connector consent + scoped machine API key).

## What it does

- **Call events → ticket activity timeline**: the connector pulls the 3CX
  call journal (`GET /xapi/v1/Cdr`, OData `$top`/`$skip` paging) and pushes
  each completed call as a Meridian call log — idempotent per provider call
  id (`uq_voip_call_id` replays the stored row). A call whose id sits in the
  ticket context map pushes WITH `ticket_id`, so the call event lands on
  the ticket activity timeline.
- **Click-to-call context actions**: `ThreecxSync::dial` originates one
  outbound call through `POST /xapi/v1/Users/Pbx.MakeCall`
  (`{dn, destination}` — source extension/device + destination number) and
  returns the call id that seeds the ticket context the sync consumes when
  the call completes. Call state flows back asynchronously through 3CX's
  notifications, outside this adapter.

## Auth (3CX side)

3CX V20 uses OAuth2 client-credentials: a client application minted in the
3CX Management Console (XAPI + Call Control scopes) authenticates against
`POST /connect/token`; tokens expire after one hour and the shared grant
cache refreshes a minute early, so a long journal pull never rides an
expiring token. XAPI calls ride the bearer token under
`{fqdn}/xapi/v1/...`.

## Auth (Meridian side)

The machine API key rides `x-api-key` on every ingest call (the VoIP ingest
surface authenticates scoped machine keys; the org's `voip` connector
installation is the consent, the key the transport). The org admin installs
the `voip` connector (config `{ "access_token": "…", "provider": "…" }` —
presence only; the connector crate never reads it) and issues the machine
key with the Tickets:View grant the ingest route requires.

## None mode (honest posture)

- **No Meridian connection** (`NoneCallLogSink`): the journal pull still
  happens, every mapped call counts as "would have pushed", and ZERO
  Meridian I/O is performed.
- **No 3CX client credentials / PBX unreachable**: the adapter reports a
  typed `ThreecxError::Vendor` pull failure — 3CX is a self-hosted PBX, so
  without credentials there is no token, no call control, and nothing to
  pull. The Meridian side stays fully functional: the VoIP ingest seam
  accepts call rows from any provider, and every push the adapter WOULD
  have made is counted in the run report.
- No webhook signature surface: 3CX call events arrive via the journal
  pull (the XAPI is an authenticated pull API), not an inbound webhook —
  so this adapter has no unsigned inbound hop to verify. The PBX-to-Meridian
  trust boundary is the two-sided credential pair (PBX OAuth2 grant +
  Meridian machine key).

## Call mapping

`call_id` = 3CX's CDR id (the ingest's idempotency gate); `direction`
in|out from the CDR's IN/OUT marker (case-insensitive, common spellings
accepted); `external_number` = the caller on an inbound call, the callee
on an outbound call; `duration_seconds` parsed from string-or-number form;
`occurred_at` from the call-start timestamp (defaults to now when the CDR
carries none). Unmappable records are REPORTED — never fabricated.

## Tests

Conformance runs against contract-shaped stub servers (axum) reproducing
the token grant, bearer-gated journal paging, MakeCall, and Meridian's
VoIP-ingest wire shape (asserted byte-exact, matching the regenerated
OpenAPI pin — the endpoint landed in the pins with Wave 22 lane 1, so the
typed client carries it and nothing here hand-rolls beyond it). No live
PBX, no live tenant.
