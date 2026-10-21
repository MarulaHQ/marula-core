-- Query vectors for the real-embedding run.
--
-- The synthetic fixture derives queries from stored rows, so a query is its
-- own nearest neighbour at distance zero. Real retrieval is asymmetric: the
-- corpus holds document embeddings and a user's search string is embedded with
-- the query prefix, so the nearest row is close but not identical. This file
-- loads those query embeddings, which the corpus builder produced from the
-- subject lines of stored documents using `task: search result | query: ...`.

\set ON_ERROR_STOP on

SET ROLE marula_app;

TRUNCATE bench.queries;

CREATE TEMP TABLE staged_q (tenant text NOT NULL, q int NOT NULL, qtext text NOT NULL);
\copy staged_q FROM '/tmp/p52-queries.tsv'

INSERT INTO bench.queries SELECT tenant, q, qtext FROM staged_q ORDER BY tenant, q;

DO $$
DECLARE n int;
BEGIN
  SELECT count(*) INTO n FROM bench.queries;
  IF n <> 40 THEN
    RAISE EXCEPTION 'query set is % rows, expected 40 (two tenants x 20)', n;
  END IF;
  IF (SELECT count(DISTINCT tenant) FROM bench.queries) <> 2 THEN
    RAISE EXCEPTION 'query set covers % tenants, expected 2',
      (SELECT count(DISTINCT tenant) FROM bench.queries);
  END IF;
END $$;

RESET ROLE;
