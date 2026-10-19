# Deploy descriptor

The machine-readable description of how marula-core is deployed: one image,
a set of profiles, and the full catalogue of runtime parameters. It is the
interface marula-hq consumes so it can render a deployment without knowing
marula-core's internals.

## The command

```sh
marula deploy describe --json                 # the whole descriptor
marula deploy describe --json --profile all-in-one
marula deploy describe                        # human summary
marula deploy describe --json --out d.json    # write to a file
```

The JSON is stable: keys are ordered, and every field is described below.
Versioning is two fields:

- `schema_version` — the descriptor shape. Bump only on a breaking change, so a
  consumer can refuse a value it does not know.
- `core_version` — the marula-core release this descriptor was built from. A
  consumer pins its fixture against this.

## The shape

```
{
  "schema_version": 1,
  "core_version": "0.1.0",
  "image": {
    "name": "marula-core",
    "roles": ["core-api", "worker", "marula-core-cli"],
    "dockerfile": "deploy/Dockerfile.rust",
    "role_arg": "BIN"
  },
  "profiles": [
    {
      "name": "all-in-one",
      "title": "...", "description": "...",
      "services": [{ "name": "core-api", "role": "core-api",
                     "ports": [8080, 8082],
                     "needs": ["postgres", "redis"] }],
      "sets": { "MARULA_ALL_IN_ONE": "1", "...": "..." }
    }
  ],
  "params": [
    { "name": "JWT_SECRET", "group": "secrets", "kind": "secret",
      "required": true, "default": null, "secret": true,
      "description": "..." }
  ]
}
```

One image runs any role. The `role_arg` selects the binary (`BIN`), so a
deployment builds or pulls the image once and runs each role as a process.

`profiles` are the named deployment shapes. `split` runs core-api and
worker as separate processes. `all-in-one` runs one core-api that hosts the
worker loops and serves the SSE live-push stream, and `sets` carries the presets that make it
so plus the small-install cadence profile.

`params` is the runtime surface: every env var a deployment may set, its group,
its kind (`string`, `integer`, `boolean`, `url`, `secret`), whether the process
refuses to boot without it (`required`), its default when it has one, and
whether a consumer must mask it (`secret`). Flatten the active profile's `sets`
over these defaults and the result is the environment to run.

## Regenerating the profiles

`deploy/profiles/*.yaml` is generated from the same code and kept in step by a
drift test:

```sh
marula deploy profiles            # rewrites deploy/profiles/*.yaml
```

Run it after changing the descriptor. The test fails when the checked-in files
fall behind, so the human-readable copy cannot disagree with the JSON.

## What this is not

marula-core states the interface; it makes no rendering decisions and holds no
hq logic. How a parameter is presented, which are grouped into a form, and how
a profile is chosen are all on the consumer's side.
