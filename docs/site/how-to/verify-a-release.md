---
description: "Check a Marula release before you install it: the signed checksum list, the archive, the extracted tree, and the image digests."
---

# Verify a release before you install it

> **Audience:** site operator — you downloaded a release, or you are about to
> point an install at one, and you want to know the bytes are the ones the
> project published.

Four checks, cheapest first. Each answers a different question, and the first one
is the only one that is worth anything on its own: a checksum you read from the
same page that served the file proves nothing.

## 1. The signature over the checksum list

The release key signs one small list per platform. Everything below trusts that
list, so verify it first.

```sh
ssh-keygen -Y verify \
  -f release-keys/allowed_signers \
  -I releases@getmarula.com \
  -n file \
  -s SHA256SUMS-linux-x64.sig < SHA256SUMS-linux-x64
```

`Good "file" signature for releases@getmarula.com` is the pass. Anything else
means stop: the list cannot be trusted, so nothing it vouches for can either.

The trust anchor is the public key in `release-keys/allowed_signers`, with its
fingerprint written beside it. Compare that fingerprint against a second source
before you rely on it — a key you fetched from the same place as the artefact
protects you from nothing.

The CLI does the same check and can also confirm every file the list names:

```sh
marula artifact verify \
  --sums SHA256SUMS-linux-x64 \
  --signature SHA256SUMS-linux-x64.sig \
  --key release-keys/allowed_signers
```

## 2. The archive against the signed line

```sh
sha256sum marula-0.3.1-linux-x64.tar.gz        # shasum -a 256 on a Mac
grep 'marula-0.3.1-linux-x64.tar.gz' SHA256SUMS-linux-x64
```

The two hashes match or the file is not the one that was signed. The signed line
covers the archive; that is the security boundary, and it is why the list itself
is the thing the key protects.

## 3. The tree, once it is unpacked

```sh
tar -xzf marula-0.3.1-linux-x64.tar.gz
cd marula-0.3.1-linux-x64
sha256sum -c SHA256SUMS
```

This list is not signed on its own, and it does not need to be. It exists so a
release that unpacked badly reports the file by name instead of failing later at
run time. A failure here after a pass in step 2 means the unpacking or the disk
is at fault, not the download.

## 4. The image digests

A registry install pulls four images, and `release.json` names the exact digest
of each. The installer reads that field, so this check is for the person who
wants to confirm it by hand:

```sh
python3 -c "import json; print(json.load(open('release.json'))['images']['core-api'])"
# ghcr.io/marulagh/marula-core-api@sha256:…
```

Ask the registry what that reference points at:

```sh
docker buildx imagetools inspect ghcr.io/marulagh/marula-core-api@sha256:…
```

The digest it prints is the digest you asked for, or the pull fails. A tag in
that position would be the mistake this check exists to catch: a tag can be
moved, and a digest is what the release signed.

Two things worth knowing:

- **A manifest with no `images` map** is a release that published no images.
  `--images registry` refuses to plan against it and says so; `--images build`
  compiles from a checkout instead.
- **The offline bundle** carries the same images by digest, saved rather than
  pulled, so an air-gapped install runs byte-identical images to an online one.

## When a check fails

| Failed | What it means |
|---|---|
| the signature | the list, or the file you fed it, is not what the key signed — stop |
| the archive hash | the download is incomplete or altered — download it again |
| the tree list | the extraction is wrong, or the disk is — try a second directory |
| the image digest | the registry does not hold that digest — the release is not pullable as published |

None of these has a "probably fine" reading. The signed line is the claim; a
mismatch means the file is not the file.

## What this does not cover

macOS and Windows builds ship **unsigned and unnotarised** for now, so the
operating system will warn on first launch even after every check above passes.
What is true while they are unsigned: the archive's checksum is signed with the
release key, `marula artifact verify` refuses a tampered file, and the update
manifest refuses an unsigned one. What a platform signature adds is the OS
saying so without you running a verifier.

Next, if the checks pass: [install on a server](install-on-a-server.md) or
[install with the app](install-with-the-app.md).
