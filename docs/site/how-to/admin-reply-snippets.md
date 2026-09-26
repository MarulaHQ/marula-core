# How to use reply snippets

> **Audience:** both — any staff member who writes ticket replies can
> insert snippets; admins manage the library.

A reply snippet is a canned reply your team writes once and inserts
everywhere. Open the composer on any ticket, press **Snippets**, and pick
one. The snippet's tokens (`{{contact.name}}`, `{{ticket.number}}`,
`{{client.name}}` and the rest) fill in from that ticket before the text
lands in your draft. What you send never shows a raw token.

## Insert a snippet into a reply

1. Open a ticket and click into the composer (the floating composer in
   the workbench, or the notes box in the classic view).
2. Press **Snippets** in the tool row.
3. Type in the search box to narrow the list — the search matches
   snippet names and body text.
4. Click a snippet. The body is inserted with every token already
   resolved for this ticket. Edit it like any other text before sending.

Insertion never sends anything. The text lands in your draft; you review
and send as usual.

## Where snippets can be used

- **Ticket composer.** Internal notes, client-visible replies, and
  in-house updates all take snippets.
- **Client-visible replies.** These are what the client sees on the
  portal. Insertion resolves tokens first, so portal replies never
  contain unresolved `{{...}}` text.

Only staff see the picker. Clients never compose from the library.

## Manage the library

**Settings → Reply snippets** holds every snippet in your organization.

- **Add a snippet** with a short name and the body you want inserted.
  Use tokens where the text should adapt per ticket; leave them out for
  fixed text.
- **Offer on boards** — tick the boards where the snippet should appear.
  Leave every box unticked to offer it everywhere. A snippet scoped to
  one board is hidden from the picker on other boards.
- **Edit** changes the snippet in place for the whole organization.
- **Fork** duplicates a snippet into a new editable copy and remembers
  which snippet it came from. A team can keep a shared original and
  diverge a variant without losing the thread.
- **Remove** takes the snippet out of the picker. Notes already sent are
  untouched.

## Notes and limits

- Tokens come from the closed ticket vocabulary: contact, client,
  ticket, assignee, org, and user fields. A token outside that list is
  rejected when the snippet is saved, never silently left empty.
- Bodies are plain text with line breaks preserved. Names cap at 120
  characters, bodies at 20,000.
- Adding, editing, forking, or removing a snippet is an administrator
  action and lands in the audit log.
