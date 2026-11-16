---
description: "The palette's second mode answers questions with the screen as its starting context, shows its sources, and files proposals for your approval."
---

# How to ask the assistant about the screen you are on

> **Audience:** both — anyone whose role has been granted the general chat
> surface. If the **Ask** switch is not there, the grant is not there either.

The command palette searches and jumps. When an admin has granted the general
chat surface, it also answers questions with the page you are looking at as
its starting context. It is a working assistant, not a chat window bolted onto
the product: it starts from the screen you are on, says which sources it used,
and files any proposed change for your approval instead of sending it.

## Prerequisites

- The `general_chat` AI surface granted to your role or to you, and a model
  available to the workspace. An ungranted surface shows no **Ask** switch at
  all — not a disabled control and not a hint. See
  [Manage AI access](admin-ai-access.md).
- A signed-in session. The grant is read per person, and the conversation is
  cleared when a different person signs in on the same browser.

## 1. Open the palette and switch to Ask

Press `Cmd+K` on macOS or `Ctrl+K` elsewhere. Search mode is the default:

- `Cmd/Ctrl+K` opens the palette.
- **Tab** from the search field switches to **Ask** when the surface is
  granted. The Search/Ask switch beside the field does the same thing.
- `Enter` sends the question in Ask mode.

The first time you switch, the palette shows a context chip naming the screen
the conversation starts from — for example **This screen: Ticket MC-1042 ·
Acme**. The chip is a starting point, not a fence. When a question needs more
than the screen in front of you, the assistant can read other records you are
allowed to see, such as another ticket or another client. Every read appears
in the sources list. What it cannot do is read past your own permissions:
your access is the ceiling.

## 2. Ask a question with an answer in the product

Good questions name the thing on screen and the decision you are making:

- "What does this status mean for the SLA clock?"
- "Draft a customer-visible reply explaining the delay."
- "Which of these line items is not covered by the contract?"
- "What should I check before I close this ticket?"

Questions about something else get a short answer and a pointer to the right
screen, not a guess.

## 3. Read the sources

Under an answer, the panel lists what it used: the screen context, each tool
call, and any proposed change. A tool row reads like **Used search** with the
outcome in plain words. A proposal row is dashed and says **Waits for your
approval. Nothing is sent.**

That distinction is the trust boundary:

- An answer is text. It changes nothing.
- A tool call is a read with a recorded outcome.
- A proposal is a draft change that waits in the approval queue on the agent
  platform. Approving it is a separate act, on a separate surface, by a person
  with the permission.

If the runner stops early, the panel says why in the same words the ledger
uses: the step limit was reached, the workspace's hard cap was reached,
verification held the answer, or a tool call failed. A stopped turn is not a
silent truncation.

## 4. Keep the conversation while you work

The palette is 680 pixels wide, which is fine for one answer and cramped for a
long one. Press **Open as a panel** and the conversation moves to a panel
anchored above the timer bar. The bar stays visible underneath, so a running
timer is never hidden.

- `Escape` collapses the panel to a tab. The tab carries the first line of the
  last answer, so you can see what you were reading.
- Press the tab to reopen the conversation.
- Closing the panel does not save a transcript. The conversation lives for the
  session. A sign-out or a different user on the same browser clears it.

## 5. When the answer is not available

Two refusals are ordinary, and both name their cause:

- **The surface is not granted.** There is no Ask switch. Ask an admin to grant
  the surface in **Settings → AI**. The grant belongs to a role, not to the
  browser.
- **No model is available.** The switch can be granted while the workspace has
  no usable model. The first turn answers with a refusal. The fix is on the
  provider or local-model side, not in the chat panel.

If a turn fails with **No answer came back**, retry once. A repeated failure is
worth reporting through [Get help inside Marula](get-help.md) with the screen
name and what you asked.

## What you learned

- `Cmd/Ctrl+K` opens the palette, and `Tab` switches to Ask when the surface is
  granted.
- The chip is the starting point, not the limit. The assistant can read what
  you may read, and the sources list shows every read.
- Sources are shown, and a proposed change waits for approval elsewhere.
- The panel keeps the conversation above the work bar. It is not a saved
  transcript.
- No switch means no grant. A refusal after a granted turn usually means no
  model.

## Where to go next

- [Manage AI access](admin-ai-access.md) — the master switch, the surface
  grant, and the cap.
- [AI: deny-by-default](../explanation/ai-deny-by-default.md) — why an
  ungranted helper is absent rather than refused.
- [Use the keyboard shortcuts](staff-keyboard-shortcuts.md) — the rest of the
  palette's keys.
- [Your first agent run](../tutorials/first-agent-run.md) — the approval queue
  a proposal lands in.
