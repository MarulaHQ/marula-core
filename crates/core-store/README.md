# core-store

The `ObjectStore` edge for Meridian (§4.7 traits-at-the-edge): one trait,
several adapters, zero vendor leakage.

## Purpose

- **`ObjectStore` trait** — `put` / streamed `get` / idempotent `delete` /
  `presign_get` (presign-or-stream), with content type and byte size
  first-class on every object.
- **Adapters**
  - `local::LocalFsStore` — the documented **filesystem none mode**: a
    self-hosted Meridian needs NO object store service; attachments land
    under a data directory with atomic publishes and `.meta.json`
    sidecars.
  - `s3::S3Store` — any S3-compatible endpoint (MinIO in the compose
    stage2 profile, AWS S3, Cloudflare R2) via the Apache Arrow
    `object_store` crate (chosen over hand-rolled SigV4 for reviewed
    crypto and over `aws-sdk-s3` for dependency weight; its types never
    escape this crate).
  - `memory::MemoryStore` / `memory::RecordingStore` — test doubles with
    an operation log; `MemoryStore::arm_write_failures()` drives
    fail-closed outage tests.
- Shared key validation: keys are `[A-Za-z0-9/._-]`, no empty/`.`/`..`
  segments — traversal is structurally impossible on every adapter.
- Attachment keys are content-addressed:
  `org/<org_id>/attachments/<sha256>` — dedup is a pure lookup, integrity
  is checkable by re-hashing bytes on read.

## None modes (standalone-first doctrine)

| Capability | Selection | Behavior |
|---|---|---|
| Object storage | `OBJECT_STORE=local` (default) | Bytes persist under `OBJECT_STORE_LOCAL_DIR`; fully functional single-node deployments. |
| Object storage | unset store env (`none`) | Upload/download endpoints degrade cleanly; nothing durable is written. |
| Object storage | `OBJECT_STORE=s3` | Requires `S3_BUCKET`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY`; `S3_ENDPOINT` selects MinIO/R2 (empty = real AWS). |

## Non-purpose

No authorization (core-api owns it), no quotas, no malware scanning
(`core-mail::ScanProvider` gates before storing), no multipart/resumable
uploads, no bucket lifecycle or replication, no presigned URLs handed to
browsers in v1 — downloads stream through the API behind authz.

## Operator setup

### local (default)

```sh
OBJECT_STORE=local
OBJECT_STORE_LOCAL_DIR=/var/lib/meridian/objects   # default ./data/objects
```

Back this directory up like any other stateful volume.

### MinIO (compose stage2 profile)

```sh
docker compose -f deploy/docker-compose.yml --profile stage2 up minio -d
docker run --rm --network meridian_default minio/mc \
  sh -c "mc alias set m http://minio:9000 meridian meridian-dev-minio && mc mb -p m/meridian"
# then:
OBJECT_STORE=s3 S3_ENDPOINT=http://localhost:9000 S3_BUCKET=meridian \
S3_REGION=us-east-1 S3_ACCESS_KEY_ID=meridian S3_SECRET_ACCESS_KEY=meridian-dev-minio
```

Buckets are NOT auto-created; create once with `mc mb` (or your IaC).

### Cloudflare R2 / AWS S3

Same as MinIO with `S3_ENDPOINT=https://<account>.r2.cloudflarestorage.com`
(R2) or unset (AWS), plus real credentials. Least-privilege key scoped to
the single Meridian bucket.

## Manual smoke (once per release)

With MinIO running locally on :9000:

```sh
OBJECT_STORE=s3 S3_ENDPOINT=http://localhost:9000 S3_BUCKET=meridian \
S3_REGION=us-east-1 S3_ACCESS_KEY_ID=meridian S3_SECRET_ACCESS_KEY=meridian-dev-minio \
cargo test -p core-store --features live-s3 -- --ignored s3_live
```
