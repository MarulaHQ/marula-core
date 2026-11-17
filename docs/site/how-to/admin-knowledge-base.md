---
description: "How to publish a runbook, decide who can read it, keep a client's infrastructure records current, and make the portal show only what you meant to share."
---

# How to write knowledge and keep client records current

> **Audience:** admin — you own the category tree, the publish decision and the
> quality of what the team reads.

The knowledge base has two jobs: publishing runbooks the team can follow, and
holding the facts about each client that a ticket cannot. Both are only useful
when they are current. This page covers the authoring side, the portal gate,
and the records that feed the expiry rollup and the completeness reading.

## Prerequisites

- `kb:create` to write articles and categories, and `kb:edit` to change one.
- `secrets:edit` to create or reveal a credential, and to manage secret
  category grants.
- At least one category before the first article. Categories order the tree
  and decide which articles the client portal may show.

## 1. Write an article

Open **Knowledge → Articles** and press **New article**. The editor saves as
you type. There is no Save button to chase. The fields that matter are:

- **Title** — required. A title like "VPN will not connect" is easier to find
  than "Networking notes".
- **Body** — the runbook itself. Put the steps in the order a person performs
  them, and say what to check before escalating.
- **Publish state** — the row under the editor, covered next.

The ledger behind the editor has views for **All**, **Drafts**, **Published**
and **Archived**, filters, column choices and an export. A draft that is not
finished is easy to find again. That is what the Drafts view is for.

## 2. Decide whether the client sees it

Publication is three conditions, and all three must be true before an article
appears in the client portal:

1. **Status** is **Published**. A client-facing draft is still a draft.
2. **Portal** on the article is **Client-facing**.
3. The article's **category** is also client-facing.

The article row states which condition is missing: published but category
staff-only, client-facing but unpublished, or published and visible. Read that
line before you tell a client to look for something.

That third condition is the one people miss. The category is a gate the article
sits behind: while the category is staff-only, even a client-facing article
stays invisible.

## 3. Keep the category tree honest

Press **New category** on the articles page to add one. Categories order the
tree and gate portal visibility for everything on their shelf. Edit a category
when its audience changes rather than moving every article by hand.

Keep the tree usable with two rules:

- Name a category after the job, not the department. "Connectivity" ages
  better than "Tier 2".
- Archive an article when it is replaced. Deleting it loses the history that
  explains why the new one exists.

## 4. Store a credential, and control who can reveal it

Open **Knowledge → Vault**. The scope switch chooses the organisation vault or
one client's vault. The two do not mix. Create a category first, then a record
inside it.

A record carries its kind, fields, rotation date, expiry and links. The value
is sealed. The list never shows it. Revealing requires `secrets:edit` and
writes a `secret.revealed` audit entry. A restricted category chain grants
access by role or by person, and a chain you cannot access is hidden from the
list and from search rather than shown as a locked row.

**Category grants** opens the grant dialog for the categories you can manage.
An org-level seed category can be forked into a client's scope so the client
gets their own copy without two records pretending to be the same credential.

Two habits prevent most vault problems:

- Rotate the record when the credential changes, so the rotation date means
  something and the stale count is true.
- Put the credential in the narrowest category that still lets the right
  people work. A shared "All credentials" shelf defeats the grant model.

## 5. Record the client's infrastructure

The client hub links to five record types. Each one feeds the expiry rollup
and the completeness reading, so a blank shelf is visible instead of silent.

| Record | What to put in it | What it feeds |
|---|---|---|
| **Networks** | Subnets with CIDR, gateway and VLAN, plus the DHCP range. Address reservations with hostname and what uses them | The network completeness leg, and the answer to "is this address free" |
| **Racks** | Rack size and location, then each device with its start unit, height and face | The physical layout a site visit needs |
| **Domains** | Registrar, registration expiry, TLS expiry and issuer, DNS provider and hosting | Domain and certificate rows in Expirations |
| **Procedures** | Ordered steps, which are required, the owner and whether the client may read it | The runbook for recurring work |
| **Credentials** | Kind, fields, rotation and expiry | Rotation rows in Expirations and the vault glance |

The list pages read these records, and some nested work is editable: subnets
and reservations on a network, devices in a rack. Networks and racks also have
a detail page that draws the layout. Creating a new top-level rack, network,
domain or procedure is not on the reading surface yet. Those records arrive
through the API. Do not describe a button that is not there: the page's empty
state says what the record is for, and the API reference has the write shape.

## 6. Work the completeness reading

Each client's completeness is eight checks: contacts, sites, devices, some
documentation, a credential, a network, a domain and a procedure. The card
shows the score and names the gaps.

Use it as a prompt, not a target. A client with no rack and no network may be a
pure SaaS customer. The gap is still worth a sentence in the review note so the
next person does not go looking. The note and the owner live in the client hub
header, and they are the difference between a blank shelf and a known blank
shelf.

## 7. Keep dates owned

**Knowledge → Expirations** gathers domain registrations, TLS certificates,
document reviews and warranties, plus credential rotation. A date is only useful
when it has an owner. An unowned date is a task nobody has accepted.

Check the 90-day window weekly, and the 365-day window at each quarterly
review. The window control is on the page, so you can widen it without
recounting by hand.

## What you learned

- An article reaches the portal only when it is published, marked
  client-facing, and sits in a client-facing category.
- The category gates portal visibility, and the article must pass its own
  checks as well.
- Credentials are sealed, revealed with an explicit grant, and audited.
- Networks, racks, domains, procedures and credentials feed both the expiry
  rollup and the completeness reading.
- Top-level infrastructure records are API-created in this build. The reading
  and nested editing are on the page.

## Where to go next

- [Find a runbook, a credential and a client's records](staff-knowledge-base.md) —
  the reader's side of the same shelves.
- [Knowledge-base categories and the secret vault](../explanation/kb-class-categories.md) —
  the grant model in one page.
- [Export your organization's data](export-data.md) — what leaves with a bundle.
- [Provenance vs audit](../explanation/provenance-vs-audit.md) — how a reveal
  is recorded.
