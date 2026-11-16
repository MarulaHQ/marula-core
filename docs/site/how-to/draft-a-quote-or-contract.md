---
description: "Say what was agreed and Marula drafts the quote or contract. Check its reading, edit in the ordinary builder, and nothing is saved until you do."
---

# How to draft a quote or contract from your notes

> **Audience:** both — whoever writes quotes and contracts. An admin grants the
> `quote_authoring` or `contract_authoring` surface before the door appears.

Most quotes start as a conversation, a page of meeting notes or a renewal
someone described on a call. **Describe it** turns those words into a draft in
the same builder you already use. It reads the words back to you before it
builds, prices from your catalogue and past quotes, and never saves anything on
its own.

## Prerequisites

- Permission to create a quote or a contract.
- The relevant AI surface granted: `quote_authoring` on **Sales → Quotes**, or
  `contract_authoring` on **Contracts**, plus a model available to the
  workspace. Without the grant, none of this renders — no button, no hint, no
  empty panel.
- A client in the workspace, if the words name one. A draft can be built
  without a matched client, but the client field is then yours to fill.

## 1. Open the door

On **Sales → Quotes** or **Contracts**, press **Describe it**. It is also the
third choice under the split **New** control, beside **From a template** and
**Start with an empty page**. All three end in the same builder.

The panel is 440 pixels and docked, not a full screen. You can keep the
underlying list in view while you describe the work.

## 2. Give it the words

The composer takes three kinds of source, and you can combine them:

- **Typed notes** — the account, the scope, the term and the price, in your own
  words.
- **Talk** — dictate the summary. The panel shows **Listening** while it
  records, and you can pause, resume or stop and review.
- **Upload notes** — a document from the call. It is read as text, not attached
  to the quote.

Short, specific descriptions work better than polished prose. These are the
shapes the panel expects:

- "Quote 10 laptops for Kestrel, installed by 20 Nov"
- "Managed Silver for Fernwood Pharmacy, 12 months from 1 Nov, with the users
  and devices they listed"
- "Retainer, 20 hours a month, P1 in 2 hours, Net 14"
- "Replace six access points at Acme's Cape Town HQ, survey first, weekend
  install"

For a contract, describe the agreement rather than the wording: who it is for,
what it covers, how long it runs and what it costs. The clause text comes from
your approved clause library, not from the model.

## 3. Check what it understood

Before it builds, the panel shows its reading: the client it found, the facts it
matched, the questions it still has and the assumptions it would make. The line
above the reading is the important one: **Check this before I build. It takes
ten seconds.**

Read the assumptions as if a colleague wrote them. The user and device counts are
facts. "Assumes weekday installation" is an assumption, and it belongs in the
draft only if it is true. If the client match is wrong, fix it here rather than
after the draft exists.

The panel also names what it is checking — the client, what to quote, pricing
from your catalogue and past quotes, budget and terms. Those are reads against
your own data, not invented figures.

## 4. Build the draft

Press **Draft it**. The build takes about twenty seconds and shows its stages.
**Keep working** closes the panel without cancelling the request; the result is
waiting when you come back.

Nothing is saved at this point. The panel says so in the failure path too:
**Nothing was saved. Your notes are still here.** If the build fails, your words
are not lost and a retry starts from the same notes.

## 5. Review and save in the builder

The result opens in the ordinary quote or contract builder. That builder is the
only write path, and you are still the person who saves.

- **Quote** — check the title, currency, validity, line items and notes. The
  draft carries its assumptions so you can remove the ones that are wrong.
- **Contract** — check the contract type, billing frequency and value, plus the
  term and effective date. The clauses are references to your approved library,
  so a variant that needs review says so.

Line prices the draft took from your catalogue or past quotes are still your
prices to confirm. A price the model inferred is marked as assumed, and an
assumed quantity is too. Change them before you save; the draft is a first
pass, not a quotation.

## 6. When it refuses

Refusals are ordinary:

- **No surface.** The grant is missing. An admin enables it in **Settings → AI**;
  the panel renders nothing until then.
- **No model.** The surface is granted but the workspace has no usable model.
  The first build is refused with that reason.
- **Not enough words.** A description with no scope or no price cannot become a
  draft. Add the missing fact, then try again.

In all three cases nothing was written. A quote or contract exists only after
you save it in the builder.

## What you learned

- **Describe it** is one door to the same builder, available only when the
  surface is granted and a model is usable.
- Type, dictate or upload the notes. Specific facts beat polished prose.
- The panel shows its reading and its assumptions before it builds anything.
- The build never saves. You edit and save in the ordinary builder.
- Prices and quantities the draft inferred are marked as assumed and need your
  confirmation.

## Where to go next

- [Price a service contract](admin-contract-types.md) — the agreement types and
  what each one bills.
- [Approve or decline a quote](client-quote-approval.md) — the client's side of
  the quote you send.
- [Bill project work](project-billing.md) — when the agreement is delivered as a
  project rather than a ticket.
- [Manage AI access](admin-ai-access.md) — grant the authoring surface and set
  the cap.
