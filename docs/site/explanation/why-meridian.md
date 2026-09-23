# Why we're building Meridian

> The canonical positioning narrative is [`MANIFESTO.md`](../../../MANIFESTO.md).
> Per the standing decision, it is quoted **verbatim** below — never
> re-synthesized, re-toned, or paraphrased. The public website quotes it the
> same way.

## The narrative, verbatim

> # Why we're building Meridian
>
> I nearly got fired over time entry.
>
> That's the honest place to start. The work was getting done. Tickets
> closed, clients served, fires put out. What I couldn't do was fight the
> software and win. Working a note into a ticket took four separate screens.
> Time entry moved slower than the fixes it described, and the hours kept
> slipping until somebody noticed. Dispatch had what a scheduler actually
> needs, buried behind tabs nobody reopens on purpose. Everyone around me
> was good at their job. The tools made all of us look like the problem.
>
> There had to be a better way. People say that a lot in this industry, and
> then go back to the system. I decided to actually build it.
>
> A ticket should take one screen, not three. Notes, timer, conversation,
> calendar live in the same view, because they are the same job. The timer
> follows you across tickets and stops cleanly, and the rough notes become a
> proper time entry.
>
> And when you're mid-ticket and you need the answer from the docs, you
> don't have to go search somewhere else. You ask. The agent reads from your
> knowledge base and the client's own documentation. It knows who the ticket
> is for, what is actually in that client's environment, and how your
> company handles this kind of problem.
>
> The agent isn't a chatbot bolted to the side - it's that old L3 who's been
> here since the company was a three-man shop. Dispatch gets the same boost,
> with intelligent suggestions based on what's actually happening with a
> ticket. Humans need to make the human decisions, but AI makes the
> mechanical things go faster.
>
> The core is open source. Host it yourself, fork it, break it, rebuild it.
> You can leave with your whole system on any given day and you can extend
> it any which way and make it entirely your own. I know the Rewst guy in
> your office is salivating thinking about the possibilities.
>
> Honestly, I just don't want to have to see another ADHD-brain tech suffer
> like I have. If no tech ever has to see that PSA that starts with a 'C'
> ever again, we'll have succeeded.

## How the narrative shows up in the product

- **One screen, not three** — the instant-first interaction doctrine
  ([explanation](instant-first.md)) is the engineering form of the manifesto's
  first promise.
- **The agent reads your docs** — the agent platform spec
  ([`docs/10-agent-platform.md`](../../10-agent-platform.md)) turns "you
  ask" into skills, provenance, safety invariants.
- **Leave any day** — the [upgrade compatibility
  policy](upgrade-policy.md) and the [read model](../reference/read-model.md)
  are the mechanical form of ownership: your data, your views, your exit.
