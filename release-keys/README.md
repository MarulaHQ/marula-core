# Release keys

`allowed_signers` is the trust anchor for release artifacts: the signature over a
release's `SHA256SUMS` and the signature on a release tag are both checked against
the key line in this file.

It is empty on purpose. Before the first release the maintainer generates the
release key, registers it, and commits its public half:

```sh
ssh-keygen -t ed25519 -C releases@getmarula.com -f ~/.ssh/marula-release
printf 'releases@getmarula.com %s\n' "$(cat ~/.ssh/marula-release.pub)" \
  >> release-keys/allowed_signers
```

The private half is held as the `MARULA_CHECKSUM_SIGNING_KEY` Actions secret and
nowhere else. The signing job in `.github/workflows/release-installer.yml`
refuses to publish while this file has no key line, because an artifact nobody
can verify should not exist.

Rotation keeps the previous line for one release, so artifacts signed before the
rotation still verify, then removes it.
