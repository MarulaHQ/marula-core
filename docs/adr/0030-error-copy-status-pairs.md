# ADR 0030: Error copy follows the status pair; destructive red is for destructive actions

Date: 2026-10-27 · Status: Accepted, sweep open · Ruling: Wave 62 register triage

## Context

The token set carries two reds and they were being used for the same job.

- `--m-destructive` (`#B42318` light, `#FB8A80` dark) is the solid mark: the P1
  chip, the SLA dot, the button that deletes something.
- The danger status pair (`#8F1C13` on `#FBE1DE` light, `#F0977F` on `#4A1A18`
  dark) is the tinted pair described in `docs/theme.md` §3.4.

Error text landed on both. The kit's breach red is `--m-status-danger-fg`, while
`.m-assistant-error` and the invalid-field border take `--m-destructive`. Both
clear AA, so the colour checker passes either way and nothing forced a decision.

The cost is semantic rather than visual. When a failed save and a delete button
are the same red, the colour stops meaning "destructive action" and starts
meaning "something is wrong", and the next screen has no way to say which one it
meant.

## Decision

1. Copy about a failure uses the danger status pair. That covers an alert, a
   refused action, and a field that will not validate.
2. The solid destructive red is reserved for the control that performs a
   destructive action: delete, revoke, purge, suspend. It marks what the button
   will do, not what a message reports.
3. The accent never carries either meaning. Ember stays interactive-only.

## Scope

Web and native. Portal surfaces keep their pinned theme and follow the same rule.
Their local duplicate of the pair retires with the rest of the portal overrides.

## Implementation status

The decision is recorded. The sweep is not done. Approximately fifty
`role="alert"` call sites still read whichever token the component was written
with, and the two that disagree are named in the live tracker's W63-E item
("Error messaging carries two reds"). `docs/theme.md` already states the breach
pair is not the destructive red (§3.4), so the sweep brings the call sites to the
sentence the design file already carries.

## Enforcement

The token conformance gate rejects a colour outside the set, which keeps this
from regressing to a third red. It cannot tell the two existing reds apart at a
call site, so the rule is held by review until the sweep lands.
