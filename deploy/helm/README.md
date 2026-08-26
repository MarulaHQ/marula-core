# Helm chart — scaffolded, unimplemented (TASKLIST Phase 3 release pipeline)

Purpose: values-driven chart for self-hosted Kubernetes installs mirroring
deploy/docker-compose.yml services (pgvector, redis, core-api, realtime,
worker; optional observability + mailpit profiles). Digest-pinned images per
release engineering policy.

Non-purpose: not an operator dashboard, no cross-env secret management beyond
documented values.
