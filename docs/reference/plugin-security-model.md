# Plugin security model

**Decision of record:** [ADR 0023](../adr/0023-plugin-runtime-isolation.md)
**Code:** `crates/plugin-host/src/{isolation,runtime,exec,abi,egress}.rs`
**Verified by:** `crates/plugin-host/tests/` — the conformance and security battery (W53-8)

A plugin is untrusted code with a signed identity. The identity is what an operator consents to,
and the code is what the sandbox contains. This page states the threats, the defences, and the things
that cannot happen, so a reviewer can check a claim against a test instead of a paragraph.

## The trust boundaries

| Boundary | On one side | On the other | Enforced by |
| --- | --- | --- | --- |
| Parent and worker | core-api, the transaction, the tenant identity | a child process running the guest | the frame protocol, plus the parent's own re-validation |
| Worker and guest | the wasmtime store | the module's linear memory | the engine's `StoreLimits`, fuel metering and epoch interruption |
| Guest and data | host calls | the invocation's `(org_id, install_id)` | the store derives the tenant; the guest names a label, never an org |
| Guest and network | `http_request` | a consented egress profile | capability, then profile, then allowlist, screened twice |
| Guest and credentials | the request | the vault | the credential is resolved and used parent-side, so only the guest's URL and body cross |

## What the parent decides, and what the worker may not

The parent runs every gate before a byte of the invocation exists:
`authorize_invocation` checks the interface version, the catalog revocation, the install's state,
and the grant, and resolves the capability onto the existing permission matrix. The frame it sends
carries the resolved grants, the declared event types and the consented egress scopes.

The worker performs no authorization. It re-validates the frame into a state that starts empty,
refuses a frame that does not validate whole, and can run only the module whose sha256 the parent
named. The claim worth stating plainly: **a bug in the worker can run a module it should not have,
and it cannot decide that a host call is allowed.** A host call the parent's own copy of the manifest
does not permit is refused again at the parent, on the invocation's transaction.

## The limits

Every number below is a constant in `crates/plugin-host/src/abi.rs`, so the executor, the host
functions and the guest SDK cannot drift apart.

| Limit | Value | What it stops |
| --- | --- | --- |
| Fuel per invocation | 200,000,000 | a CPU-bound loop, at the exact cap, in about 200 ms of compiled code |
| Wall clock per invocation | 250 ms, checked every 50 ms | a guest that weaves around the fuel meter |
| Instance memory | 64 MiB, `trap_on_grow_failure` | a growth loop; a failed grow is a trap, not a `-1` the guest can ignore |
| Output per invocation | 1 MiB, checked before the copy | a guest that claims `u32::MAX` costs the host nothing |
| Input per invocation | 1 MiB | an oversized payload, refused before the sandbox is built |
| Log | 4 KiB per line, 100 lines | a log flood |
| Host calls | 64 per invocation | a guest that loops on KV to extend its window |
| Worker resident set | 256 MiB, Linux only | a worker that grows: it answers, then retires |
| Worker exchange | 500 ms, plus a 5 s transport budget when the manifest declares egress | a worker that ignored its own deadline |
| Per-install rate | 300 a minute, burst 60 | one install consuming the queue |
| Concurrency | one invocation per org, one per install, one per worker | a plugin's own read-modify-write interleaving with itself |
| Quarantine | 3 consecutive worker failures | an artifact that takes its worker down every call |

Three of these deserve the reasoning, because the number is a judgement rather than a round figure.

**Fuel and the clock are two caps on purpose.** Fuel is exact and cheap to check. The epoch is the
hard stop for whatever the meter does not catch. Which one fires first depends on instruction
throughput, and the conformance corpus allows either at the invocation level while pinning each
cap separately with a narrowed limit. A page that promised "fuel always fires first" would be
wrong on some builds.

**64 host calls is a quarter of the guest's window.** At a generous millisecond per call that is 64
ms of a 250 ms budget, so the cap refuses before the parent's 500 ms timeout does. That matters
because a timeout kills the worker and counts a strike towards quarantine, while a refusal is a
clean answer the guest can read and stop on.

**Quarantine is per install, not per plugin.** One install's artifact taking its worker down must
not stop another org's install of the same plugin, and it must not be attributed to the catalog
entry. A guest trap is not a strike at all: the mechanism worked and the answer was wrong, so
quarantining for it would let any plugin stop itself for every caller.

## What a hostile plugin can do

The battery in `crates/plugin-host/tests/` is the evidence, and each row names the test that fails
if the defence goes away.

| Attempt | Answer | Test |
| --- | --- | --- |
| loop forever | `plugin_fuel_exhausted` or `plugin_wall_clock_exceeded`, worker survives | `bombs.rs` |
| grow memory without bound | `plugin_guest_trap` | `bombs.rs` |
| recurse without bound | `plugin_guest_trap` | corpus |
| trap explicitly | `plugin_guest_trap`, worker survives, no strike | `containment.rs` |
| claim a 4 GiB reply | `plugin_output_too_large`, nothing copied | `bombs.rs` |
| flood the log | lines dropped, invocation continues | `bombs.rs` |
| return a garbage pointer | `plugin_abi_frame_invalid` | `boundary_fuzz.rs` |
| send a malformed frame | refused by the codec, never a panic | `boundary_fuzz.rs` |
| import WASI | module refused at load | corpus |
| call a host entry that does not exist | module refused at load | corpus |
| call `http_request` with no grant | `plugin_capability_not_granted`, no socket | corpus, `escalation.rs` |
| call it with a grant but no consent | `plugin_egress_profile_missing` | corpus |
| name another org's data | impossible: the tenant comes from the invocation | `kv_sql.rs` |
| put capabilities in the payload | ignored; the grant set is the install's | `escalation.rs` |
| return a different effect | `plugin_effect_not_consented`, nothing written | `escalation.rs` |
| take the worker down repeatedly | replaced, then quarantined after three | `containment.rs` |

## What cannot happen

- A guest reading or writing another org's KV, tickets or audit trail. The tenant is derived from
  the invocation, not supplied by the guest.
- A guest opening a socket. The worker holds no HTTP client, and the request crosses a pipe.
- A guest reading a credential. Credentials resolve parent-side, and the guest sends a URL, a method,
  a header map and a body.
- A guest raising its own privileges. The grant set is the install row's, verbatim.
- A guest choosing its effect. The manifest consents one effect. A guest that names another gets
  `plugin_effect_not_consented` and no write.
- A refused invocation disappearing. One audit row is written whatever happened, by the parent,
  in the caller's transaction.
- A crash becoming an outage. A dead worker is one `plugin_worker_failed` reply and a replacement.

## The audit row

One row per attempt, action `plugin.invoke`, entity `plugin_entry`, in the caller's transaction:

| Detail key | Meaning |
| --- | --- |
| `outcome` | `ok`, `refused` or `worker_failed` |
| `reason` | the refusal string, or null on success |
| `capability` | the capability the surface required |
| `invocation_id` | the correlation id the reply echoed |
| `fuel_used` | the guest's spend; null when the worker died |
| `duration_us` | the parent's own measurement |
| `worker` | `marula-plugin-worker:<pid>`, `declarative`, or null when no worker was reached |

The actor is the install, not the caller: `Actor::Plugin(<install id>)`. An attempt with no
resolvable install is attributed to `system`, never to the caller-supplied entry key, so a refused
attempt cannot choose who it is blamed on.

## Threat model: what this does not defend against

Writing the gaps down is cheaper than discovering them in an incident.

- **A malicious publisher with a trusted key.** The signature proves who published, not that the
  code is safe. The capability grant and the sandbox are what limit it, and consent is where a
  human reads what it asks for.
- **A defect in wasmtime.** The sandbox is memory-safe by construction, not crash-safe. A
  wasmtime or Cranelift defect could take the worker down. The pool then replaces it and the
  install earns a strike.
- **Data a granted capability legitimately reaches.** A plugin granted `tickets.read` can read
  every ticket in the org. The model limits what a plugin may reach, never which rows inside a
  capability it reads.
- **Side channels.** The 250 ms wall clock, the fuel figure and the refusal codes are all
  observable by the guest. They leak the machine's speed and the presence of grants, not org data.
- **Exfiltration through a consented profile.** An allowlisted host still receives whatever the
  guest sends it. Consent is the control, and `secret_get` stays deferred precisely because
  vault-plus-egress is a one-call exfiltration composition.
