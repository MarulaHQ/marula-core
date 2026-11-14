# app-bridges

The Slack and Microsoft Teams bridge for Marula, plus the Rewst workflow
trigger. Apache-2.0, written against public APIs only: the vendor's own
documented endpoints on one side, Marula's public API and the callback
endpoints in Core on the other.

## What it does

- **Outbound notifications.** One typed send call per vendor, the same
  `NotificationPayload` shape for both, so a notification route picks an
  adapter by kind and never forks a payload.
- **Ticket cards.** `card::TicketCard` holds six facts, and
  `render_slack` / `render_teams` turn it into Block Kit and an Adaptive
  Card. One builder, two renderers: the same ticket reads the same in both
  products because the data is shared, not the layout.
- **Inbound callbacks, verification half.** `slack_inbound` verifies Slack's
  signed requests and reads an interaction payload. `teams_inbound` verifies
  a Bot Framework token and reads an activity. Neither decides what a button
  does. That decision belongs to Core's action registry, so a new action on a
  card never reaches this crate.
- **Rewst trigger.** `rewst::RewstBridge` posts an HMAC-signed envelope to an
  org webhook URL to start a workflow run.

## What it does not do

- **No OAuth handshake.** The operator provisions the app and supplies the
  credentials. This crate consumes them.
- **No action execution.** It verifies and parses. Core resolves the action,
  checks the pressing user's permissions, and writes the audit row.
- **No vendor-identity linking.** Mapping a Slack or Teams account to a
  Marula user is an admin act on `user_external_identities`, not something a
  callback does to itself.
- **No Rewst results path.** The trigger posts and requires a 2xx. A run's
  outcome has no way back into Marula today. The note at the end says what that would need.

## Consent scopes

| Bridge | Scopes | Meaning |
|---|---|---|
| Slack, Teams | `notifications.forward` | post messages into channels the operator chose |
| Rewst | `workflows.trigger` | start a workflow run over the org webhook |

## Outbound contracts

| Vendor | Mechanism | Endpoint | Body |
|---|---|---|---|
| Slack (chat) | incoming webhook | the URL from the install | `{"text": ...}` |
| Slack (card) | `chat.postMessage` | `https://slack.com/api/chat.postMessage` | `channel`, `text`, `blocks` |
| Teams (chat) | workflow webhook | the URL from the install | one Adaptive Card attachment |
| Teams (card) | Bot Framework | `{serviceUrl}/v3/conversations/{id}/activities` | the Adaptive Card message |

A Slack webhook answers `200` with an error string for a payload it does not
like, so a success status alone is not a success: the sender reads the body
and turns `missing_type` or `invalid_payload` into a typed refusal. The same
rule applies to `chat.postMessage`, whose `ok: false` is the real answer.

## The inbound callback contract

Core mounts four endpoints. This crate supplies the verification and the
payload types behind them.

| Endpoint | Caller | Authentication |
|---|---|---|
| `POST /api/v1/connectors/slack/interactive` | Slack button press | `v0` HMAC-SHA256 |
| `POST /api/v1/connectors/slack/commands` | Slack slash command | `v0` HMAC-SHA256 |
| `POST /api/v1/connectors/teams/interactive` | Bot Framework activity | Bot Framework JWT |
| `POST /api/v1/connectors/teams/messages` | messaging-extension query | Bot Framework JWT |

**The signature is the authentication.** None of these endpoints sits behind
a Marula session, because a chat channel has no session to present. Two rules
follow, and both are load-bearing:

1. **The handler reads the raw bytes before parsing.** Slack signs
   `v0:{timestamp}:{raw_body}`. Re-serialising the JSON first would change
   key order, spacing or escaping, and the digest would no longer match the
   body it claims to cover. `slack_inbound::verify` therefore takes `&[u8]`.
2. **A body field selects a candidate connection. It never authenticates
   one.** The signing secret belongs to the connection, so the connection has
   to be found before the signature can be checked, and the only thing naming
   it is a body field (Slack `team.id`, Teams `recipient.id`). Core uses that
   field as a lookup key only, then proceeds on a signature match. A forged
   selector picks a different secret and fails. If two connections verify the
   same callback, Core refuses rather than guessing, because the secret
   cannot tell two tenants apart.

### The replay window

`X-Slack-Request-Timestamp` is part of the signed string, and it is also
compared with the clock. A request more than **five minutes** away from now,
in either direction, is refused even when the signature is genuine: an old
body stays valid forever otherwise. The comparison is constant-time
(`hmac::Mac::verify_slice`), and the only length check is over a public value.

Every authentication failure answers one typed 401 with one sentence. Which
check failed is written to Core's action ledger, where an operator reads it,
and never to the caller, who would learn what to adjust.

### Idempotency

Slack re-delivers a callback it did not see answered within three seconds, up
to three times, with byte-identical bodies. Core keys each press on the
SHA-256 of the request bytes, so a retry is a replay of the first answer
rather than a second press. Failed authentications are keyed separately, so a
transient refusal cannot consume the digest a later genuine delivery needs.

### Teams tokens

Microsoft's documented validation applies: the signature is checked against
the key the published OpenID configuration names for the token's `kid`,
`exp` and `nbf` are enforced, the issuer must be the Bot Framework service or
the tenant's Azure AD issuer, the audience must be this bot's app id, and the
`serviceurl` claim must match the activity's `serviceUrl`. Keys are fetched
over the network in production, so verification sits behind the
`BotFrameworkAuth` trait and a test injects its own.

## How a press is authenticated and attributed

1. The signature proves the **connection**. Core resolves the candidate from
   the unauthenticated selector and verifies with that connection's secret.
2. The vendor user id (`payload.user.id`, or the activity's `from.aid`) is
   resolved through `user_external_identities` to a Marula user. An unmapped
   account is a typed refusal that names the link step.
3. The Marula user's principal is rebuilt from their current roles, and the
   action's own gate runs again. A press is never an org-wide capability, and
   it is never performed as somebody else.
4. One ledger row records the connection, channel, message, vendor user,
   resolved user, action and result.

The action id travels in the payload verbatim (`marula.take` on Slack,
`data.marulaAction` on Teams), so both products resolve the same registry
entry and a new action is one entry in Core rather than a match arm here.

## Operator configuration

| Bridge | What has to be supplied |
|---|---|
| Slack inbound | the app's **signing secret**, stored in the connection's sealed credentials as `signing_secret` |
| Slack outbound | a **bot token** with `chat:write`, stored as `bot_token` |
| Teams inbound | the bot's **application id**, in the connection's non-secret settings as `teams_bot_app_id` |
| Teams outbound | a **bot token**, stored as `bot_token`, and a destination that carries the Bot Framework `service_url` |
| Rewst | the org webhook URL and its shared secret |

A connection missing its secret fails with a typed configuration refusal
before a socket opens, not with a vendor error afterwards.

## Testing

The suite runs against stubs, never a live tenant:

- `slack_inbound::sign` is public so a test (or a fixture recorder) can
  produce a signature the production verifier accepts. The signature tests
  cover the scheme, a re-serialised body, the replay boundary, a wrong secret
  and a tampered body.
- `BotFrameworkAuth` is a trait. `teams_inbound` ships its tests with a stub
  implementation, and Core's flow tests install one through
  `connector_inbound::install_bot_auth`, so no test fetches Microsoft's keys.
- Card rendering is asserted by value, in both renderers, from one
  `TicketCard`.
- Conformance against loopback mock servers covers the outbound senders, as
  it does for every connector in this repository.

```bash
cargo test -p app-bridges --lib
cargo test -p app-bridges --test app_bridges_conformance
```

## Rewst, and what is missing

`RewstBridge::trigger` posts the envelope and requires a 2xx. It returns no
run id, and Marula has no endpoint that accepts a Rewst run result. The
bridge starts a workflow and cannot learn how it ended. That is the current
contract, not an oversight in this crate: a results path needs a Core-side
receiver, a run reference to match against, and a decision about what a
returned result may write. The customer-facing promise covers Slack and
Teams, so this stays a known gap until a later unit takes it.
