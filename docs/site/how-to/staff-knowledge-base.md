---
description: "The knowledge base is organised by client, not article. Here is where to find a runbook, a network record, a domain date and a credential, without guessing."
---

# How to find a runbook, a credential and a client's records

> **Audience:** both — technicians look things up here all day, and admins keep
> the shelves stocked.

The knowledge base answers "what do we know about this client" before it
answers "which article". Every client has a page that gathers their runbooks,
documents, devices, network, racks, domains, procedures and credentials. Start
there when you are picking up unfamiliar work. Use search when you already know
the name of the thing you need.

## Prerequisites

- The `kb:view` permission. It is part of every staff role that can open a
  ticket.
- To reveal a credential, `secrets:edit`, plus access to the credential's
  category. Without both, the record is not shown to you at all.
- A client you are allowed to see. Client access narrows everything here, and
  a client outside your access has no card, no search hit and no hub.

## 1. Start with the client card

Open **Knowledge** in the left rail. The home page is a client directory, not
an article list. Each card carries the counts that tell you whether the
documentation is worth trusting:

- sites, devices, documents, articles, credentials, racks, networks, domains
  and procedures
- credentials that have gone stale, and dates that are about to run out
- a completeness reading out of eight, with the missing pieces named in words
  rather than shown as a bare score

The eight checks are contacts, sites, devices, some documentation, a stored
credential, a network, a domain, and a procedure. A client that scores five
has three named gaps, and the card says which ones. Use that as a work list,
not as a judgement: a small client may have no racks at all.

Below the client cards sits the organisation's own knowledge: shared articles
and templates, plus the org vault. Those are yours regardless of which client you
opened.

## 2. Search every store at once

The search field at the top of the knowledge home looks across clients,
documents, devices, configurations, credentials and articles. Scope chips
narrow it to one store (**All**, **Clients**, **Documents**,
**Configurations**, **Secrets**), and a client filter narrows it further.

Search comes back short for two ordinary reasons:

- Search needs at least two characters. A single letter returns nothing rather
  than the whole workspace.
- A credential appears as its record only: name, category, client and dates.
  The value is not in the index and never appears in a result. Open the record
  and reveal it if you hold the grant.

Search respects the same permissions as the lists. A restricted category chain
you cannot open does not appear as an empty result or a locked row. It is
absent. If a colleague can find something you cannot, the difference is a
grant, not a search setting.

## 3. Read the client hub

Open a client card to reach its hub. The header tells you who owns the
documentation and when it was last touched. Under it:

- **Sections** — articles, documents, devices, credentials, domains, networks,
  racks, procedures, sites and contacts, each with a count that links through.
- **Expirations** — the dates inside the next 90 days that belong to this
  client.
- **Recent changes** — the newest writes, with who made them.
- **Vault glance** — how many credentials exist and how many are stale, with
  no values.

This is the page to read before a site visit. It shows what exists and what is
missing in the same view, so you know whether to trust the runbook you are
about to follow.

## 4. Follow a procedure

**Procedures** are runbooks with ordered steps. Open one and you get:

- the steps, numbered, with optional steps marked as optional
- a body that explains why the steps are in that order
- the owner, the version and whether the client can read it

Ticking a step in the reading view does not record anything. The procedure is a
template for the job, not a checklist instance. The record of what you did
belongs on the ticket. If a step is wrong, tell the owner rather than
working around it silently — the next person will follow the same runbook.

## 5. Read the infrastructure records

Three record types answer the questions that usually start a call.

| Surface | What it answers |
|---|---|
| **Networks** | Which subnets and VLANs exist, their gateways, DHCP ranges, and which addresses are already reserved and for what |
| **Racks** | What is installed, in which unit, on which face, and what power and cooling it draws |
| **Domains** | Who the registrar is, when registration and the TLS certificate expire, where DNS and hosting live, and whether Marula manages the DNS |

Each record links to the others. A rack layout names the device. A network
reservation names the address and what uses it. A domain names its certificate
dates. The linked-records strip on each page is the fastest way to move between
them.

A date you did not record reads **Not recorded**, never a guess. An asset
warranty is typed by a person rather than read from a registry, and the record
marks the difference.

## 6. Check what is about to expire

Open **Knowledge → Expirations**. One list gathers every date that matters:
domain registrations, TLS certificates, document reviews, asset warranties and
credential rotation. Choose a window (30, 60, 90, 180 or 365 days) and a
kind if you want one.

Blush means soon and red means the date has passed, but the row also carries
the date and the words, so colour is never the only cue. Each row names the
owner and where the date came from. An expiry with no owner is a task waiting
for one.

## 7. Reveal a credential

Credentials live in the **Secret vault**, either at organisation level or per
client. The category tree is the permission boundary: a category you cannot
open is absent from the list, not greyed out.

Revealing is a deliberate act:

1. Open the record. You see its fields, last rotation, expiry and links — not
   the secret.
2. Press **Reveal**. The value appears in a dialog.
3. Use it, then close the dialog.

The reveal writes an audit entry naming you and the record. The value is not
kept in page state after the dialog closes, and it is not copied into notes.
Reveal needs `secrets:edit`. A restricted category also needs an explicit
grant. If the button is missing, that is the grant, not a broken page.

## What you learned

- The knowledge home is organised by client, and its completeness reading names
  what is missing.
- One search covers every documentation store, and credentials appear as
  records only.
- Procedures are runbooks to follow, and ticking a step is not stored.
- Networks, racks and domains answer different infrastructure questions and
  link to each other.
- Expirations are one list with an owner and a source for every date.
- A credential reveal is audited, and a category you cannot open does not
  exist for you.

## Where to go next

- [Write knowledge and keep client records current](admin-knowledge-base.md) —
  the authoring and administration side.
- [Knowledge-base categories and the secret vault](../explanation/kb-class-categories.md) —
  why the grant model is one model for both.
- [Provenance vs audit](../explanation/provenance-vs-audit.md) — what a reveal
  entry can and cannot tell you.
- [Get help inside Marula](get-help.md) — the contextual help drawer if a
  surface is not where this page says it is.
