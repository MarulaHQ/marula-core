# Release images

The installer can run in two shapes: build the application images from the
checkout, or pull them from a registry pinned by digest. This page is about the
second one — what the release publishes, how it names it, and what an install
consumes.

`--images build` compiles this tree on the target machine. That is right for a
developer and wrong for a release: a bundle ships compose files and the wizard,
not the Rust and Next.js sources, so a build-shaped install cannot run from one.
Registry images are what make a release bundle installable, which is why this is
a release concern rather than an installer option.

## Names

Four images, one repository prefix, one image per service:

| Service | Image |
|---|---|
| `core-api` | `ghcr.io/marulahq/marula-core-api` |
| `worker` | `ghcr.io/marulahq/marula-worker` |
| `web` | `ghcr.io/marulahq/marula-web` |
| `migrate` | `ghcr.io/marulahq/marula-migrate` |

`marulahq` is the organisation's login in lowercase, which is the only form GHCR
accepts. `ghcr.io/marulahq` is public, so a self-hoster pulls without a token. A
private registry works too: `--image-base` takes any prefix, and the operator
runs `docker login` once.

The four are built from `deploy/Dockerfile.rust` with a different `BIN` argument,
except `web`, which builds the Next.js bundle.

## Tags and digests

**A digest is the identity. A tag is a convenience.**

- Every release pushes `<version>` (`0.4.0`) and nothing else that an installer
  reads. The tag exists so a person can `docker run` something recognisable.
- `latest` may be published for convenience and is **never** consumed by the
  installer or by a manifest. An install that followed `latest` would change
  behaviour without a release.
- The manifest records the digest per service. `--images registry` refuses to
  plan without a digest for every service it would start, because a tag can be
  moved and a digest is what the release signed.

`ghcr.io/…@sha256:…` is what lands in `marula.images.yml`, and the compose
override drops the base file's `build:` with `build: !reset null` so Compose
pulls the pinned image instead of compiling one.

## What the pipeline does, per release

1. Build `linux/amd64` and `linux/arm64` on their own native runners, one
   `buildx` run per image and architecture, with
   `--provenance=true --sbom=true`, and push each leg by tag.
2. Join each service's legs into one manifest with
   `docker buildx imagetools create`, then read its digest back:
   `docker buildx imagetools inspect <ref> --format '{{json .Manifest.Digest}}'`.
3. Write the digests into the release manifest as `images`, so the signed
   document names exactly what it published. `images.mjs` writes them either into
   an existing manifest or, with `--digests images.json`, as the bare map the
   assembler takes:

   ```sh
   node scripts/release/images.mjs --version 0.4.0 --digests dist/release/images.json
   node scripts/release/assemble.mjs --manifest --version 0.4.0 \
     --out dist/release --images dist/release/images.json
   ```

   The order is not a preference. `SHA256SUMS` covers `release.json` and the
   release key signs that list, so a digest merged in after the manifest is
   written sits outside both, and a bundle would then publish digests nobody
   signed.
4. Sign the manifest's `SHA256SUMS` as the update engine already does. The
   manifest signature is what makes the digests trustworthy, so the two must be
   signed together and committed in the same job.
5. Publish the bundle. The bundle's compose files and the manifest travel with
   it; the images do not, because a registry holds them.

An air-gapped install uses the same digests: `docker save` writes the image the
digest names into the offline archive, so the offline path and the online path
install byte-identical images.

## Publishing them

`.github/workflows/release-images.yml` is the job that does steps 1 and 2 above:
one runner per image and architecture, GHCR as the destination. The `amd64`
legs run on `ubuntu-24.04` and the `arm64` legs on `ubuntu-24.04-arm`, so both
architectures compile natively; a per-service publish leg joins them with
`buildx imagetools create` and reads the resulting digest. It writes the digests
as a workflow artifact named `marula-images`, which the release workflow's
`marula-*` download picks up as `dist/release/images.json`. The manifest step
merges that before it writes `SHA256SUMS`, and the release key signs the list —
so the digests are covered by the signature rather than added beside it.

A tagged release runs it, before the platform builds. Each platform archive
carries the digests in its own `release.json`, because someone can download and
install that archive without the rest of the release set. `release-installer.yml`
calls this workflow as its `images` job, and the build jobs run after it, so the
digest map is an artifact of the same run as the manifests that have to name it.
Running the workflow by hand still works, and is the cheap way to prove a push
without cutting a tag:

```sh
gh workflow run release-images.yml -f version=0.3.1
```

Each architecture builds on its own native runner, so the job no longer pays for
emulation; the `arm64` half is as fast as the `amd64` one. A manual run of the
release workflow rehearses the same path without a tag:
`gh workflow run release-installer.yml -f images=true`.

A failed images job fails the release. `publish` accepts a skipped images job,
which is what a non-tag run has, but not a failed one: a manifest with no
`images` map describes a bundle that installs only from the offline tarball.

One thing the job cannot do for you:

- **Make the packages public.** GHCR creates private packages for a token push,
  even in a public repository. Until all four are set to public in their package
  settings, a self-hoster's `docker pull` fails with `unauthorized` and no
  explanation. The run summary repeats this.

## Retention

- Keep every image that a published manifest names, forever. A digest an older
  manifest references is what lets someone install that release again.
- Everything else may be pruned. GHCR keeps versions indefinitely by default, so
  pruning is a deliberate act with a list of digests to keep, derived from the
  manifests.
- Never delete a digest and then re-tag the same version. Rebuilding `0.4.0` and
  pushing it again breaks every install that pinned the old digest, and the tag
  now means something the manifest does not.

## Trust, stated plainly

The chain is: the manifest is signed, the manifest names digests, the installer
pins those digests. That is enough to know the bytes are the ones the project
released.

Two gaps are honest to name:

- **The images are not signed individually.** A tool like `cosign` would let
  someone verify an image without the manifest. The digest pin covers the install
  path, so this is a hardening step rather than a hole in the release.
- **`ghcr.io` is a third party.** A self-hoster who prefers their own registry
  mirrors the four images by digest and passes their prefix to `--image-base`.
  The digest check still runs, so a mirror cannot substitute a different image
  silently.

## Today, and what is missing

Built: `images.mjs` (build, push, read the digest back, write the map), the
assembler's `--images` merge, the installer reading a bundle's `images` instead
of four typed flags, the refusals that name a missing prefix or a service with
no digest, and the `images` job in `release-installer.yml` that runs this
workflow on every tag.

Still open:

1. A published set. Until a run pushes one, no bundle's manifest carries an
   `images` map and `--images registry` correctly refuses against it.
2. `cosign` signing over the images, if the mirroring story above is not enough.

## Cloudflare is not the image registry

The update channel lives on Cloudflare: a Worker behind `updates.getmarula.com`
serves the signed manifest, and the artifacts behind it sit in R2 under
`artifacts.getmarula.com`. Those are the app and its update payload, not images.

There is no OCI registry in that setup, and Cloudflare does not offer one, so
`--images registry` cannot pull from `getmarula.com`. The four images live in
GHCR. A self-hoster who wants neither can mirror by digest and pass their own
prefix; that path is described under Trust above.
