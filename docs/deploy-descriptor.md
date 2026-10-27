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
  "schema_version": 2,
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
      "sets": { "MARULA_ALL_IN_ONE": "1", "...": "..." },
      "floors": { "cores": 4, "ram_mb": 8192, "disk_gb": 30 },
      "services": [{ "name": "core-api", "role": "core-api",
                     "ports": [8080, 8082],
                     "needs": ["postgres", "redis"] }]
    }
  ],
  "params": [
    { "name": "JWT_SECRET", "group": "secrets", "kind": "secret",
      "required": true, "default": null, "secret": true,
      "description": "...",
      "consequence": "...", "default_rationale": null,
      "requires": [], "conflicts": [] }
  ]
}
```

One image runs any role. The `role_arg` selects the binary (`BIN`), so a
deployment builds or pulls the image once and runs each role as a process.

`profiles` are the named deployment shapes. `split` runs core-api and
worker as separate processes. `all-in-one` runs one core-api that hosts the
worker loops and serves the SSE live-push stream, and `sets` carries the presets that make it
so plus the small-install cadence profile. `floors` is the machine the shape
needs: cores, mebibytes of RAM and gibibytes of free disk. It is a lower bound,
and it is checked against the profile's own presets — a profile that presets a
local embedding model its floors cannot host is a contradiction the descriptor
check refuses.

`params` is the runtime surface: every env var a deployment may set, its group,
its kind (`string`, `integer`, `boolean`, `url`, `secret`), whether the process
refuses to boot without it (`required`), its default when it has one, and
whether a consumer must mask it (`secret`). Each one also carries:

- `consequence` — what changes for the operator when the lever moves, in plain
  language. This is the sentence a wizard renders, and a consumer must not write
  a second explanation.
- `default_rationale` — why the default is that value. `null` only when the
  lever has no default.
- `requires` — other levers that must be set alongside this one when it is in
  use. Empty for a lever that stands alone.
- `conflicts` — levers that must not be set at the same time.

Flatten the active profile's `sets` over these defaults and the result is the
environment to run.

Three sections carry decisions that are more than a default value:

- `retries` — the preset matrix, with each subsystem's budget and the levers
  that override it.
- `embedding` — the model this instance runs, its capacity floor, and how a
  re-index is paced.
- `retrieval` — the vector leg's mode (`exact`, `iterative`, `index`), the
  requirement each mode adds, the measured consequence of each, and the
  per-tenant embedding distribution. `resolved.mode` is what this environment
  selects and `resolved.refusal` is the sentence the boot would print when that
  mode cannot run. When a database is reachable the section carries a live probe.
  When it is not, `resolved.probe` says `not_run` rather than claiming the mode
  can run.

## The self-check

The descriptor is checked before it is published. `check` in
`crates/marula-core-cli/src/deploy_check.rs` refuses:

- a parameter with no `consequence`, one shorter than eight words, or one that
  merely restates `description`.
- a default with no `default_rationale`.
- a `requires` or `conflicts` entry naming a lever the descriptor does not
  publish, a lever naming itself, a conflict only one side declares, or a lever
  that both requires and conflicts with the same name.
- a profile that declares no floors, or whose presets select a local embedding
  model its own floors cannot host (a cloud model carries no local floor, so it
  is always allowed).

`marula deploy describe --json` refuses to print a descriptor that fails the
check, and `marula deploy profiles --write` refuses to write one. The
human summary prints the violations instead of failing, so an operator staring
at a broken build still sees what is wrong.

## Regenerating the profiles

`deploy/profiles/*.yaml` is generated from the same code and kept in step by a
drift test:

```sh
marula deploy profiles            # rewrites deploy/profiles/*.yaml
```

Run it after changing the descriptor. The test fails when the checked-in files
fall behind, so the human-readable copy cannot disagree with the JSON.

## What this is not

marula-core states the interface. It makes no rendering decisions and holds no
hq logic. How a parameter is presented, which are grouped into a form, and how
a profile is chosen are all on the consumer's side.
