# Release images without the release trigger

The release workflow is `.github/workflows/release.yml.disabled` on purpose: a
release publishes artifacts to a registry, and that is the maintainer's
decision. Until it is re-enabled, this is the operator path for building the
four images once, pushing them to a private registry, and deploying by digest.
It mirrors the disabled workflow's gates rather than replacing them.

This is the interim path. If the trigger is re-enabled, the GHCR flow in the
workflow becomes the path of record and this page is the fallback for an
air-gapped or private-registry install.

## Images

| Image | Dockerfile | Build argument |
|---|---|---|
| `marula-core-api` | `deploy/Dockerfile.rust` | `BIN=core-api` |
| `marula-worker` | `deploy/Dockerfile.rust` | `BIN=worker` |
| `marula-core-cli` | `deploy/Dockerfile.rust` | `BIN=marula-core-cli` |
| `marula-web` | `deploy/Dockerfile.web` | none |

## Build once, on a builder host

Use the release commit, not a working tree. `REV` is the git SHA and `TAG` is a
human label; the digest is what a deployment pins.

```bash
REV=$(git rev-parse --short=12 HEAD)
TAG="0.1.0-$REV"
REGISTRY=registry.example.internal/marula

docker buildx build --platform linux/amd64 \
  -f deploy/Dockerfile.rust --build-arg BIN=core-api \
  --provenance=mode=max --sbom=true \
  -t "$REGISTRY/marula-core-api:$TAG" --push .

docker buildx build --platform linux/amd64 \
  -f deploy/Dockerfile.rust --build-arg BIN=worker \
  --provenance=mode=max --sbom=true \
  -t "$REGISTRY/marula-worker:$TAG" --push .

docker buildx build --platform linux/amd64 \
  -f deploy/Dockerfile.rust --build-arg BIN=marula-core-cli \
  --provenance=mode=max --sbom=true \
  -t "$REGISTRY/marula-core-cli:$TAG" --push .

docker buildx build --platform linux/amd64 \
  -f deploy/Dockerfile.web \
  --provenance=mode=max --sbom=true \
  -t "$REGISTRY/marula-web:$TAG" --push .
```

`buildx` writes an SBOM and a provenance attestation next to each image when the
builder supports it (`--sbom=true --provenance=mode=max`). Keep the SBOMs as
artifacts too, so they survive a registry that does not expose attestations:

```bash
mkdir -p sbom
for image in marula-core-api marula-worker marula-core-cli marula-web; do
  syft "$REGISTRY/$image:$TAG" -o spdx-json > "sbom/$image.spdx.json"
done
```

## The gates the workflow would have run

Run these before pushing, or immediately after, and fail the build on the same
conditions:

```bash
# A HIGH or CRITICAL fixable vulnerability fails the image.
for image in marula-core-api marula-worker marula-core-cli marula-web; do
  trivy image --ignore-unfixed --severity HIGH,CRITICAL --exit-code 1 \
    "$REGISTRY/$image:$TAG"
done

# A hadolint error fails the Dockerfile.
hadolint deploy/Dockerfile.rust deploy/Dockerfile.web
```

The workflow also packages the Helm chart and prints a digest manifest. On the
interim path, produce the manifest yourself and keep it with the SBOMs:

```bash
for image in marula-core-api marula-worker marula-core-cli marula-web; do
  digest=$(docker buildx imagetools inspect "$REGISTRY/$image:$TAG" \
    --format '{{json .Manifest.Digest}}' | tr -d '"')
  echo "$image=$digest"
done | tee "sbom/digests-$TAG.txt"
```

## Deploy by digest

A tag can move. A digest cannot. Pin every image in the deployment values:

```yaml
# compose override, or the Helm values file
services:
  core-api:
    image: registry.example.internal/marula/marula-core-api@sha256:<digest>
  worker:
    image: registry.example.internal/marula/marula-worker@sha256:<digest>
  web:
    image: registry.example.internal/marula/marula-web@sha256:<digest>
```

For the chart, set the same `@sha256:` references and run its
`digestPinningEnforced` gate. Keep the digest manifest beside the deployed
values; it is the record of what ran.

## Record which path was used

The landing record for a deployment says one of two things:

- **Interim**: images built on a builder host, pushed to the private registry,
  deployed by digest, with the SBOM, provenance and digest manifest paths.
- **GHCR**: the release trigger was re-enabled by the maintainer and the
  workflow pushed and attested the images. The workflow's summary is the
  record.

Do not claim the GHCR path without a successful workflow run, and do not treat a
local `docker build` as a release.
