# Valkey down, unreachable, or out of memory

**Reached from:** `MarulaWorkerNotReporting`, `MarulaConsumerPelDepthHigh`, `MarulaOutboxLagHigh`

## What the alert means

The event stream lives in Valkey, under the service name `redis` in the compose
file. When it is unreachable the relay cannot publish, so the outbox stops being
marked published and every consumer stops receiving. When it is reachable but
full, the failure is louder: the shipped profile runs `--maxmemory 192mb` with
`--maxmemory-policy noeviction`, so a write past the ceiling is refused rather
than evicting anything.

The second case is the one worth recognising quickly. A memory ceiling does not
degrade the stream, it takes the relay and every consumer down together, and the
symptom in the worker log is a write error rather than a timeout.

## First three checks

1. Is it answering at all:

   ```sh
   docker compose exec redis valkey-cli PING
   docker compose exec redis valkey-cli DBSIZE
   ```

2. Is it full, and is the stream what filled it:

   ```sh
   docker compose exec redis valkey-cli INFO memory | grep -E 'used_memory_human|maxmemory_human|maxmemory_policy'
   docker compose exec redis valkey-cli XLEN marula:events
   docker compose exec redis valkey-cli XINFO GROUPS marula:events
   ```

   Compare `used_memory_human` against the 192 MB ceiling, and `XLEN` against
   `MARULA_EVENT_STREAM_MAXLEN` (default 50,000). A stream at its bound with
   memory also at the ceiling means the bound is sized for a larger instance than
   this one.

3. Is the consumer group still there, and how far behind is it? A group that has
   vanished is a group that will be recreated at the live tail when its consumer
   starts, which means the entries in between are skipped:

   ```sh
   docker compose exec redis valkey-cli XINFO GROUPS marula:events
   docker compose exec redis valkey-cli XPENDING marula:events <group>
   ```

## Escape hatch

If the container is gone, restart it. Consumers reconnect on their own and the
relay resumes publishing unpublished outbox rows, because the outbox is the
queue and Valkey only carried the stream. A restarted Valkey loses the entries it
held unless its data volume survived, so any gap is reconciled from a REST
snapshot rather than from the stream start, which is the documented path in
`docs/worker-cadence.md`.

If it is out of memory, freeing memory is the immediate action and the only safe
freeing move is trimming entries every group has already read. Do not raise
`maxmemory` on a one-box install to make a backlog fit, and do not switch the
policy to `allkeys-lru`: an evicted stream entry is an event nobody receives and
nothing reports.

If the stream keeps reaching its bound, the fix is `MARULA_EVENT_STREAM_MAXLEN`
sized against the memory in `docs/capacity-model.md`, not a bigger number chosen
under pressure.

## Afterwards

Record which of the two failures it was. An unreachable Valkey is an
infrastructure fault and will repeat. A full one is a sizing decision that is
still pending, and it will return on the next busy day.
