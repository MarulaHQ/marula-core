// The kit's half of the ABI parity story.
//
// `abi/host-abi.v1.json` is the catalogue; `pdk/marula-abi.mjs` reads it and
// validates a `plugin.wasm` surface against it at build time. The Rust host
// proves the catalogue matches its live registry
// (`crates/plugin-host/tests/abi_catalogue.rs`); this file proves the JS reader
// and the WASM surface check agree with the catalogue for good and bad modules.
//
// Run with `node --test test/` (the package's `npm test`).

import assert from "node:assert/strict";
import test from "node:test";

import {
  ABI_NAMESPACE,
  ABI_VERSION,
  HOST_FUNCTIONS,
  LIMITS,
  QUERY_IDS,
  REFUSALS,
  inspectModule,
  refusalCode,
  requiredExports,
  validateModuleAbi,
} from "../pdk/marula-abi.mjs";

// --- a minimal WASM writer -------------------------------------------------
// Hand-rolled on purpose: a fixture that goes through a compiler would test
// the compiler. These are exactly the sections `inspectModule` reads.

const leb = (value) => {
  const out = [];
  let v = value;
  do {
    let byte = v & 0x7f;
    v >>>= 7;
    if (v !== 0) byte |= 0x80;
    out.push(byte);
  } while (v !== 0);
  return out;
};

const vec = (items) => [...leb(items.length), ...items.flat()];
const str = (value) => [...leb(Buffer.byteLength(value)), ...Buffer.from(value, "utf8")];
const section = (id, payload) => [id, ...leb(payload.length), ...payload];
const i32 = 0x7f;
const i64 = 0x7e;
const functype = (params, results) => [
  0x60,
  ...vec(params.map((param) => [param])),
  ...vec(results.map((result) => [result])),
];

const LOG_TYPE = functype([i32, i32, i32], []);
const ALLOC_TYPE = functype([i32], [i32]);
const ENTRY_TYPE = functype([i32, i32], [i64]);
const FREE_TYPE = functype([i32, i32], []);
const QUERY_TYPE = functype([i32, i32, i32], [i64]);
const VOID_TYPE = functype([], []);

const T0 = 0;
const T1 = 1;
const T2 = 2;
const T3 = 3;
const T4 = 4;
const T5 = 5;
const TYPES = [LOG_TYPE, ALLOC_TYPE, ENTRY_TYPE, FREE_TYPE, QUERY_TYPE, VOID_TYPE];

/** The three exports every guest must provide. */
const REQUIRED = [
  { name: "marula_alloc", typeIndex: T1 },
  { name: "marula_entry", typeIndex: T2 },
  { name: "marula_free", typeIndex: T3 },
];

function buildModule({ imports = [], exports = REQUIRED }) {
  // One local function per export, so an export's type is whatever the case
  // declares. Export indices are offset by the imported function count: the
  // WASM index space is imports first, then locals.
  const funcs = exports.map((entry) => entry.typeIndex);
  const importedFunctions = imports.length;
  const bytes = [0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
  bytes.push(...section(1, vec(TYPES)));
  if (imports.length > 0) {
    bytes.push(
      ...section(
        2,
        vec(
          imports.map((entry) => [
            ...str(entry.module),
            ...str(entry.name),
            0x00,
            ...leb(entry.typeIndex),
          ]),
        ),
      ),
    );
  }
  if (funcs.length > 0) {
    bytes.push(...section(3, vec(funcs.map((typeIndex) => leb(typeIndex)))));
  }
  if (exports.length > 0) {
    bytes.push(
      ...section(
        7,
        vec(
          exports.map((entry, index) => [
            ...str(entry.name),
            0x00,
            ...leb(importedFunctions + index),
          ]),
        ),
      ),
    );
  }
  if (funcs.length > 0) {
    bytes.push(...section(10, vec(funcs.map(() => [0x00, 0x0b]))));
  }
  return Buffer.from(bytes);
}

const errorsOf = (bytes) => validateModuleAbi(bytes).errors.join("\n");

// --- catalogue ------------------------------------------------------------

test("the catalogue is the v1 ABI the ADR fixes", () => {
  assert.equal(ABI_VERSION, 1);
  assert.equal(ABI_NAMESPACE, "marula");
  assert.deepEqual(requiredExports().toSorted(), ["marula_alloc", "marula_entry", "marula_free"]);
  assert.equal(HOST_FUNCTIONS.length, 9);
  assert.deepEqual(
    HOST_FUNCTIONS.map((fn) => fn.name),
    [
      "log",
      "kv_get",
      "kv_put",
      "kv_delete",
      "kv_list",
      "http_request",
      "emit_event",
      "read_model_query",
      "now",
    ],
  );
});

test("refusal codes are dense from 1 and every reason is unique", () => {
  assert.equal(REFUSALS.length, 25);
  const codes = REFUSALS.map((row) => row.code).toSorted((a, b) => a - b);
  assert.deepEqual(
    codes,
    Array.from({ length: 25 }, (_, index) => index + 1),
  );
  assert.equal(new Set(REFUSALS.map((row) => row.reason)).size, 25);
});

test("refusalCode resolves the strings an author will actually see", () => {
  assert.equal(refusalCode("plugin_capability_not_granted"), 1);
  assert.equal(refusalCode("plugin_not_implemented"), 13);
  assert.equal(refusalCode("plugin_abi_frame_invalid"), 15);
  // The one a credentialed egress call can return (W53-12).
  assert.equal(refusalCode("plugin_credential_refused"), 25);
  assert.equal(refusalCode("plugin_something_that_does_not_exist"), undefined);
});

test("the limits the kit prints are the ADR's numbers", () => {
  assert.equal(LIMITS.input_bytes, 1_048_576);
  assert.equal(LIMITS.output_bytes, 1_048_576);
  assert.equal(LIMITS.kv_list_page, 500);
  assert.equal(LIMITS.http_redirects, 0);
  assert.equal(LIMITS.wall_clock_ms, 250);
});

test("the query table is closed at four named ids", () => {
  assert.deepEqual(
    QUERY_IDS.map((row) => [row.id, row.name]),
    [
      [1, "ticket.by_id"],
      [2, "board.list"],
      [3, "client.by_id"],
      [4, "contact.by_client"],
    ],
  );
});

// --- module inspection ----------------------------------------------------

test("a module that matches the ABI passes with no errors", () => {
  const bytes = buildModule({
    imports: [
      { module: "marula", name: "log", typeIndex: T0 },
      { module: "marula", name: "read_model_query", typeIndex: T4 },
    ],
  });
  assert.deepEqual(validateModuleAbi(bytes), { ok: true, errors: [] });
});

test("the inspector reads the import and export surface", () => {
  const { imports, exports } = inspectModule(
    buildModule({ imports: [{ module: "marula", name: "now", typeIndex: T0 }] }),
  );
  assert.equal(imports.length, 1);
  assert.equal(imports[0].module, "marula");
  assert.equal(imports[0].name, "now");
  assert.equal(exports.length, 3);
  assert.deepEqual(exports.map((entry) => entry.params), [["i32"], ["i32", "i32"], ["i32", "i32"]]);
  assert.deepEqual(exports.map((entry) => entry.results), [["i32"], ["i64"], []]);
});

test("a WASI import is refused by namespace", () => {
  const errors = errorsOf(
    buildModule({ imports: [{ module: "wasi_snapshot_preview1", name: "fd_write", typeIndex: T0 }] }),
  );
  assert.match(errors, /only import namespace is "marula"/);
});

test("secret_get and ai_complete are not in the ABI", () => {
  for (const name of ["secret_get", "ai_complete"]) {
    const errors = errorsOf(buildModule({ imports: [{ module: "marula", name, typeIndex: T0 }] }));
    assert.match(errors, /not one of the nine v1 host functions/, name);
  }
});

test("an import with the wrong arity is refused with both signatures", () => {
  const errors = errorsOf(
    buildModule({ imports: [{ module: "marula", name: "log", typeIndex: T1 }] }),
  );
  assert.match(errors, /imports marula\.log as \(i32\) -> \(i32\); the ABI declares/);
});

test("a missing export is refused by name", () => {
  const errors = errorsOf(
    buildModule({ exports: REQUIRED.filter((entry) => entry.name !== "marula_entry") }),
  );
  assert.match(errors, /missing the required export marula_entry/);
});

test("an export with the wrong return type is refused", () => {
  // marula_alloc declared as `() -> ()` instead of `(i32) -> (i32)`.
  const errors = errorsOf(
    buildModule({
      exports: [
        { name: "marula_alloc", typeIndex: T5 },
        { name: "marula_entry", typeIndex: T2 },
        { name: "marula_free", typeIndex: T3 },
      ],
    }),
  );
  assert.match(errors, /marula_alloc is \(\) -> \(\); the ABI declares/);
});

test("a non-function import (a memory) is refused", () => {
  const payload = [
    ...str("marula"),
    ...str("memory"),
    0x02,
    0x00,
    ...leb(1),
  ];
  const bytes = Buffer.from([
    0x00,
    0x61,
    0x73,
    0x6d,
    0x01,
    0x00,
    0x00,
    0x00,
    ...section(1, vec(TYPES)),
    ...section(2, vec([payload])),
    ...section(3, vec(REQUIRED.map((entry) => leb(entry.typeIndex)))),
    ...section(
      7,
      vec(REQUIRED.map((entry, index) => [...str(entry.name), 0x00, ...leb(index)])),
    ),
    ...section(10, vec(REQUIRED.map(() => [0x00, 0x0b]))),
  ]);
  assert.match(errorsOf(bytes), /only functions may be imported/);
});

test("garbage is refused instead of throwing", () => {
  assert.deepEqual(validateModuleAbi(Buffer.from("not wasm at all")), {
    ok: false,
    errors: ["not a WASM module (missing \\0asm magic)"],
  });
  assert.equal(validateModuleAbi(Buffer.alloc(0)).ok, false);
});
