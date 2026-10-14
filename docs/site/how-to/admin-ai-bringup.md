# How to bring up AI with your own API keys

> **Audience:** admin — you own the org settings, and you are the one who has
> to know a key works before a customer sees the product.

Six settings decisions, in a fixed order, all on **Settings → AI**. The
cards on that page sit in the order of the steps below. Do them in this
order and the first live call works. Do them out of order and you get a
typed refusal that names the step you skipped, which is the design working
([AI: deny-by-default](../explanation/ai-deny-by-default.md)).

Screenshots of this page in both themes are in the lane evidence record,
`design/qa-p47-byok-runbook/captures/`.

## Prerequisites

- Admin access to **Settings → AI**. BYOK writes are an owner/admin
  surface: a custom role carrying admin-module permissions still cannot
  save a credential.
- The demo stack booted and seeded, and a provider key to hand. If the demo
  machine has no vendor key at all, skip to the last section and run the
  local model runtime instead.
- [How to manage AI access](admin-ai-access.md) for the posture, and
  [Local AI: transcription and embeddings on your own hardware](../reference/self-host-local-ai.md)
  for the reasoning behind operator-run models.

## 1. Turn the org master switch on

**AI features** is the org-wide master switch, and it defaults **OFF**. While
it is off, no surface can call a model, no usage ledger row is written, and
nothing about AI appears anywhere in the product. An org that leaves it off
keeps the exact pre-AI product. That is the point of the default, not an
accident of it.

Flip **Allow AI in this organisation** to On. The server stores it as the
`ai.features_enabled` org setting, and it refuses every AI edge typed before
any other check runs. Turning it off later is a panic switch that works.
The refusal happens before anything else is consulted, so there are zero
side effects to clean up.

Do this first. A grant saved while the switch is off stays valid, because
grants and the switch are independent. Nothing is audible until both are
true.

## 2. Pick the provider

Four provider ids accept a key: `openai`, `anthropic`, `ollama`, `custom`.
Pick one in the **Byok providers** card.

Two of the four have live adapters in this build, and which two matters:

| Provider | Completions | Embeddings |
| --- | --- | --- |
| `openai` | wired | wired |
| `ollama` | wired | wired |
| `anthropic` | degrades typed | degrades typed |
| `custom` | degrades typed | degrades typed |

An `anthropic` or `custom` key seals fine, but a call against it answers
with a typed "no transport for this provider" refusal instead of reaching a
model. If a demo needs one of those two, check with the platform team about
which lane wires the adapter first.

**The base URL.** Neither `ollama` nor `custom` has a per-org base-URL field
in this build. The endpoint comes from the deployment environment:

| Provider | Where the base URL comes from | Default |
| --- | --- | --- |
| `ollama` | `OLLAMA_BASE_URL` on core-api | `http://localhost:11434/v1` |
| `openai` | `OPENAI_BASE_URL` on core-api | `https://api.openai.com/v1` |

Under compose, Ollama is a service name, so set:

```sh
OLLAMA_BASE_URL=http://ollama:11434
```

`localhost` there means the core-api container itself, which is not where
Ollama runs. The same two variables point at an OpenAI-compatible gateway
when a customer proxies their models through one.

## 3. Save the key, then test it

Two cards on this page hold sealed material, and they are not the same slot.
**Byok providers** is the legacy seam, one key per provider id, for the
search embeddings leg. **Credential slot** is the matrix seam, org and
personal credentials for the completion edge, with a kind (`api_key`,
`oauth_token`, `subscription_account`) and an optional label.

Both seal at rest with the deployment's `MARULA_SECRET_KEY` (AES-256-GCM).
Neither ever renders the secret back. What comes back is a presence flag:
`has_key: true` and a timestamp. Writes ride the instant-first path, so a
retry with the same idempotency key replays the same result instead of
storing twice, and the write trail records presence only. If a key ever
shows as plaintext to anyone, stop the demo and report it.

Then press **Test**. What the button proves, honestly:

- none mode answers with the typed reason that nothing is configured;
- a missing key for the selected provider says so, by name;
- a key that fails to decrypt says it was tampered with, or that the master
  key was rotated under it;
- otherwise: "credentials verified and provider ready".

That last line means the credential is present and opens. It does not make a
vendor HTTP call in this build. A key can pass the button and still be
refused by the provider. That gap is why the next section exists.

Save the same key into the **Credential slot** when you want live
completions. Pick the provider, kind `api_key`, paste the secret, and leave
**Personal** unchecked for the org slot.

## 4. Grant the surfaces per role

A configured provider enables nothing. The **Surfaces** card is the
availability matrix, and it is deny-by-default. A surface with no grant row
is callable by nobody, and the copy on the card says exactly that. Nothing
is granted "to everyone" by a global flag.

Seeded surfaces you will see:

- **Native**: `general_chat`, `ticket_note_assist`, `doc_drafting`,
  `email_drafting`
- **Plugin**: `data_analysis`, `automation_writing`, `dashboard_generation`,
  `transcription`
- **Internal**: `embeddings_search` backs vector search and is not a grant
  target, because core calls it and never a user

To switch a surface on for a role, find its row, pick Role, pick the role,
then Grant. The change is audited and immediate. For the demo, grant
`general_chat` to the role the presenter signs in with. The probe in the
next section calls that surface.

## 5. Allowlist the model, then set the caps

**Model catalog.** The completion edge runs only (provider, model) pairs on
the enabled allowlist. Add yours with the capability tags it truly has:
`completion`, `tools`, `decision`. Tags are load-bearing. The chat edge
requires `completion` plus `tools` on every call, because the runner always
offers its read-only tool wardrobe, and a model catalogued without the right
tag is refused typed at admission, never guessed around.

**Caps.** Every configured period binds at admission over its own window.
Weekly, monthly, quarterly and yearly are each enforced like the others.
Three rules decide what happens when usage climbs:

1. A per-user cap shadows the org-wide cap of the same period.
2. Usage is cost-weighted by the catalog entry's weight, so an expensive
   model spends the budget faster.
3. At a hard cap the call is refused typed before dispatch. At a soft cap
   the call proceeds, with one warning notification per crossed period.

For a demo org, set a monthly org-wide cap whose hard limit covers the whole
session with room to spare. That keeps a runaway script from spending a
customer's budget without turning the demo down mid-call.

## 6. Decide the personal-credentials policy

The **Credential slot** card carries a **Personal (my own key)** checkbox,
disabled while the admin policy is off. The policy is the
`ai.personal_credentials_enabled` org setting, and it defaults to **off**.

Off means one org credential serves everyone. On means a user's own
credential for the same provider wins over the org's for their own calls,
with the org key as the fallback. Leave it off for a demo. An org key keeps
every presenter's call on the same billing surface, and a personal key that
expires mid-demo is a surprising way to learn which user saved one.

## Prove it before the room

The Test button checks the seal, not the provider. Before a prospect is in
the room, run the probe:

```sh
node scripts/demo-ai-probe.mjs
```

It reads the API base URL and login from the environment, with the exact
variables in the script header. It signs in, then makes two live calls. The
first is a chat completion on `general_chat` through the sealed credential,
through the same admission pipeline a real call runs: master switch, grant,
allowlist, caps, credential. The second is an embedding call against the
768-dimension contract the search vector leg enforces.

Every failure mode answers with what happened and what to do about it. The
script prints no key material at any point. Exit code 0 means both legs
passed. Non-zero means read the last line.

| What the probe says | What it means | What to do |
| --- | --- | --- |
| `ai_features_disabled` | the master switch is off | step 1 |
| `ai_surface_not_granted` | no grant covers you on that surface | step 4 |
| `ai_model_not_allowed` | the pair is not on the allowlist | step 5 |
| `ai_hard_cap_reached` | the cap bound before dispatch | raise the cap or wait for the window |
| `ai_no_credential` | no active credential for that provider | step 3 |
| provider unreachable | the base URL is wrong, or the runtime is down | step 2, check `OLLAMA_BASE_URL` |
| wrong embedding width | the model does not return 768 dimensions | pick a conforming model |

## Running with no external key at all

A demo does not have to send anything to a vendor. The `ai` compose profile
runs a local model runtime on the box:

```sh
docker compose -f deploy/docker-compose.yml --profile ai up -d ollama
docker compose -f deploy/docker-compose.yml exec ollama ollama pull whisper
docker compose -f deploy/docker-compose.yml exec ollama ollama pull nomic-embed-text
```

`nomic-embed-text` returns 768 dimensions natively, which is the width the
search vector leg requires. `whisper` covers transcription on the
OpenAI-compatible seam. Then set `OLLAMA_BASE_URL=http://ollama:11434`,
pick `ollama` as the provider, and leave the key empty. A local runtime
checks no credential, so an empty one is accepted.

Use this when the demo machine is air-gapped, or when the customer cannot
hand over a vendor key for policy reasons. It is slower on CPU and it
borrows memory, since the slot is capped at 4 GB and 2 CPUs. For a normal
hosted demo a vendor key is the better trade. The runtime publishes no host
port on purpose. Core-api reaches it by service name inside the compose
network, and nothing else can. The full local-AI posture, including why no
weights ship in a Marula image, is in
[Local AI: transcription and embeddings on your own hardware](../reference/self-host-local-ai.md).

## What you learned

- The master switch comes first. OFF is the pre-AI product, and flipping it
  off later stops every edge with zero side effects.
- `openai` and `ollama` are the two providers with live adapters.
  `anthropic` and `custom` seal a key but degrade typed until wired.
- `ollama` and `custom` take their endpoint from `OLLAMA_BASE_URL` and
  `OPENAI_BASE_URL` on the deployment, not from an org setting.
- The sealed slot never renders a key back. The Test button proves the seal,
  and the probe proves the provider.
- Grants are deny-by-default per surface. The allowlist and the caps bind at
  admission, before any dispatch.
- Personal credentials stay off by default. On, they win over the org key,
  per user.

## Related

- [How to manage AI access](admin-ai-access.md), the switch and grants as a
  posture with the four quadrants of the page
- [How to run transcription and embeddings on your own hardware](admin-local-ai-models.md),
  the local runtime in operator detail
- [AI: deny-by-default and the decision harness](../explanation/ai-deny-by-default.md),
  the reasoning behind each layer
