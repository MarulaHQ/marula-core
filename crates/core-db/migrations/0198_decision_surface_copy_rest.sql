-- Decision surface descriptions still opened with ADR citations, and one
-- native surface still described itself by its runner's ADR number. An admin
-- reading Settings → AI has no use for either: the ADR is where the decision
-- was argued, not what the surface does.
--
-- This rewrites the remaining seven seeded surfaces in the words the person
-- granting them uses. Keys, kinds and enabled state are untouched. The ADRs
-- keep their numbers in the code and the decision records.
--
-- Sentences avoid semicolons so the migration lint reads each statement as a
-- keyed row update.

UPDATE ai_surfaces SET description = 'Per-alert actionability and severity. The system only auto-acknowledges above the high thresholds you set.'
 WHERE key = 'decision_alert_triage';

UPDATE ai_surfaces SET description = 'The AI decision condition inside automation rules. Its answers feed the existing rule match, and nothing else changes.'
 WHERE key = 'decision_automation_conditions';

UPDATE ai_surfaces SET description = 'Dispatch queue assignment and escalation urgency, suggested for a dispatcher to accept.'
 WHERE key = 'decision_dispatch';

UPDATE ai_surfaces SET description = 'Inbound email and new or updated tickets, read for actionability, urgency, board and queue choice, tags, priority and frustration.'
 WHERE key = 'decision_ticket_triage';

UPDATE ai_surfaces SET description = 'A pre-flight check over the tools an agent has been granted. The check can narrow the list, never widen it.'
 WHERE key = 'decision_tool_routing';

UPDATE ai_surfaces SET description = 'Inline suggestion cards for the workbench, with a 70 to 500 ms budget and one speculative pass per trigger.'
 WHERE key = 'decision_workbench';

UPDATE ai_surfaces SET description = 'The general agent chat surface.'
 WHERE key = 'general_chat';
