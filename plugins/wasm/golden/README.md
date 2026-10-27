# golden

A pinned corpus of plugin guests and the exact answer each one must get from the
host. The conformance battery in `crates/plugin-host/tests/conformance.rs` reads
this directory and runs every module twice: once in the wasmtime sandbox, which
reports fuel and captured log lines, and once through a real worker process,
which is the transport a deployment runs.

The point is drift. A host change that moves a refusal code, a limit or a load
verdict has to show up as a diff against a table a reviewer can read, not as a
behaviour nobody notices until a customer's plugin stops working.

## What is here

| File | What it holds |
| --- | --- |
| `expectations.json` | one row per module: the guest file, the grants the invocation carries, and the verdict |
| `src/*.wat` | the guests, in WebAssembly text form |

The `.wat` files are compiled by the test with the `wat` crate, so the corpus
needs no `wasm32` target, no build step and no binary in the repository. A module
here is small enough to read and change in a review diff.

## Adding a case

1. Write the guest as `src/<name>.wat`. It needs the three exports the ABI fixes
   (`marula_alloc`, `marula_entry`, `marula_free`) and one `memory` export, and it
   may import only the `marula` namespace.
2. Add a row to `expectations.json` with a `claim` that says what the case
   proves. The claim is the sentence a reviewer reads first, so it has to be the
   guarantee, not a restatement of the file name.
3. Run `cargo test -p plugin-host --test conformance`.

A row may assert an outcome (`ok` or `refused`), a refusal string, literal output
bytes, an output length, the captured log line count, or a module that must be
rejected at load. The `reason_any_of` form exists for the one case with two
legitimate answers: a spin loop is stopped by fuel or by the wall clock depending
on instruction throughput, and both mean the invocation's allowance ran out.

## What the corpus does not cover

Limits that need a narrowed value, such as a 50,000-fuel budget or a 20 ms
clock, cannot be expressed as a table row here, because the worker always runs
the shipped numbers. Those cases live in `crates/plugin-host/tests/bombs.rs`, which builds a
runtime at the narrowed limit and pins which cap fires. The database-backed
refusals and the quarantine rule live in `escalation.rs` and `containment.rs`.
