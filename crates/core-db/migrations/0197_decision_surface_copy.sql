-- Decision surface descriptions spoke in harness vocabulary: "Noul" is the
-- internal name of a yes/no question kind, and an admin reading Settings → AI
-- has no way to know that. Rewrite the seeded descriptions in the words the
-- person deciding actually uses. Keys, kinds and enabled state are untouched;
-- only the sentence shown in the availability matrix changes.
--
-- The term stays in the code and the ADRs. It was only ever wrong on screen.

UPDATE ai_surfaces SET description = 'Tiered verification of note, email and document drafts against their source, including injection and policy yes/no checks.'
 WHERE key = 'decision_llm_verify';

UPDATE ai_surfaces SET description = 'Billable and description-matches-work yes/no checks on each time entry and invoice line at creation.'
 WHERE key = 'decision_billing_audit';

UPDATE ai_surfaces SET description = 'Residual pairwise yes/no after deterministic identity blocks; merges stay human-approved.'
 WHERE key = 'decision_record_dedupe';

UPDATE ai_surfaces SET description = 'Yes/no per top-k retrieval candidate; rides the existing Postgres index, never replaces it.'
 WHERE key = 'decision_search_rerank';

UPDATE ai_surfaces SET description = 'MSA coverage classification including after-hours surcharge yes/no; suggestion-grade, over the existing contracts schema.'
 WHERE key = 'decision_contract_scope';

UPDATE ai_surfaces SET description = 'Yes/no at agent step boundaries ("has the task satisfied its goal criteria?"), enabling early loop exit; the hard iteration cap still bounds the loop.'
 WHERE key = 'decision_loop_termination';
