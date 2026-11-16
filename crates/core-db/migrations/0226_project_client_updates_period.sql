-- One live client update per period. A repeated draft for the same week is
-- refused rather than stored twice; withheld rows sit outside the index, so a
-- week that was honestly empty can be described again once something moves.
--
-- If a deployed database already holds two live rows for the same period, this
-- stops the upgrade instead of choosing which of a PM's drafts to delete. That
-- is deliberate: duplicates have to be resolved by a person, once.
CREATE UNIQUE INDEX uq_project_client_updates_period
  ON project_client_updates (project_id, period_start, period_end)
  WHERE status <> 'withheld';
