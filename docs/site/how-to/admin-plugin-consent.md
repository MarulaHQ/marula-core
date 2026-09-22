# How to configure the plugin consent flow

> **Audience:** admin — plugin installation, consent, and cap grants are
> settings-owner security decisions.

## Prerequisites

- Admin access to **Settings→Plugins** ("Plugins & agent skills").
- Acceptance of one mental fact first: every install is a
  supply-chain decision, and the product treats it that way.

## The posture, in the surface's own words

> "Nothing executes that did not arrive digest-verified with provenance
> and pass admin consent. Installs start DISABLED; updates pin the old
> version until the consent diff is approved."

So the flow always travels: verified distribution → disabled install →
consent diff → enabled. An update is really a new consent, exactly like
the first install.

## 1. Get the plugin legitimately

The **Registry** tab is the supported distribution source, with GitHub
and zip import reserved for audited in-house plugins. Distribution itself carries the
verification: plugins arrive digest-verified with provenance — the
[distribution rationale](../explanation/ai-deny-by-default.md) intro
applies to skills and plugins alike (or see ADR 0009 for the plugin/
skill distribution model).

## 2. Install lands DISABLED

The install lands **disabled** — the state is deliberate and worth
respecting even if you "trust" the sample (the shipped
`sample-stock-ticker@1.0.0` sits enabled with its granted caps in the
demo org for study). A disabled plugin consumes nothing, grants nothing,
runs nothing.

## 3. Read the consent diff, then consent

Before enabling: read the consent diff of capabilities and surfaces the
plugin requests. This is the same deny-by-default grant decision you make
elsewhere — the plugin tells you the exact smallest list it needs and
nothing more arrives unasked. Approving consents <strong>that
version</strong>, while the very next update re-asserts the old
version and re-pins it until you approve the new diff.

## 4. Grant caps

Consent grants caps per the diff. The tabbed surface
(WASM / HTTP connectors / Agent skills / Registry / Approvals) organizes
the installed kinds. The **Approvals** tab is your pending-consent work
queue.

## 5. Revoke or retire (the same gate in reverse)

Consent is a grant, not a marriage: removing the plugin removes the
capability grants with it, and the distribution ledger's provenance stays
for review. Capability surfaces table and capability kinds are listed in
[reference/plugin capability surfaces](../reference/index.md) when you
script against them.

## What you learned

- Install start disabled, approve diff, grant caps — the same order
  every time.
- Updates re-pin the old version until the new diff is approved.
  Nothing self-upgrades runtime behavior you haven't consented to.
- For agents, the same posture governs skills (ADR 0008) — see
  [manage AI access](admin-ai-access.md) for the sibling AI surfaces.
