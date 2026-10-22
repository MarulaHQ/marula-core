// @marula-core/plugin-kit — the v1 ABI a plugin author writes against.
//
// ADR 0023 fixes the guest ABI: three exports (`marula_alloc`,
// `marula_entry`, `marula_free`), one import namespace (`marula`), nine host
// entries and no WASI. This module is the authoring surface for that
// contract, and the build step (`bin/build.mjs`) refuses a `plugin.wasm` whose
// imports or exports do not match it.
//
// The catalogue itself is `abi/host-abi.v1.json` — DATA, so the Rust host, this
// kit and a language binding can all read the same file. `crates/plugin-host`
// has a test that parses it and compares it with the live registry, so the two
// cannot drift. Numbers here are ADR 0023 Decision 4.

import { readFileSync } from "node:fs";

const ABI_PATH = new URL("../abi/host-abi.v1.json", import.meta.url);
const ABI = JSON.parse(readFileSync(ABI_PATH, "utf8"));

export const ABI_VERSION = ABI.abi_version;
export const ABI_NAMESPACE = ABI.namespace;
export const HOST_FUNCTIONS = ABI.host_functions;
export const QUERY_IDS = ABI.query_ids;
export const REFUSALS = ABI.refusals;
export const LIMITS = ABI.limits;
export const REQUIRED_EXPORTS = ABI.exports;

/** The catalogue row for a host function, or undefined. */
export const hostFunction = (name) => HOST_FUNCTIONS.find((fn) => fn.name === name);
/** The numeric code a `u32`-returning call uses for a refusal reason. */
export const refusalCode = (reason) => REFUSALS.find((r) => r.reason === reason)?.code;
/** The core-module export names a guest must provide. */
export const requiredExports = () => REQUIRED_EXPORTS.map((e) => e.name);

const wasmType = (byte) => {
  switch (byte) {
    case 0x7f:
      return "i32";
    case 0x7e:
      return "i64";
    case 0x7d:
      return "f32";
    case 0x7c:
      return "f64";
    case 0x70:
      return "funcref";
    case 0x6f:
      return "externref";
    default:
      return `0x${byte.toString(16)}`;
  }
};

function readU32(bytes, at) {
  let value = 0;
  let shift = 0;
  for (;;) {
    if (at >= bytes.length) throw new Error("truncated LEB128 integer");
    const byte = bytes[at++];
    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) break;
    shift += 7;
    if (shift > 35) throw new Error("LEB128 integer is too long");
  }
  return [value >>> 0, at];
}

function readName(bytes, at) {
  const [length, afterLength] = readU32(bytes, at);
  const end = afterLength + length;
  if (end > bytes.length) throw new Error("truncated name");
  return [Buffer.from(bytes.subarray(afterLength, end)).toString("utf8"), end];
}

function readVec(bytes, at, readOne) {
  const [count, cursor] = readU32(bytes, at);
  const items = [];
  let next = cursor;
  for (let i = 0; i < count; i += 1) {
    const [item, after] = readOne(bytes, next);
    items.push(item);
    next = after;
  }
  return [items, next];
}

/**
 * Parses the import and export surface of a core WASM module. Deliberately
 * small: five sections, no interpretation, no execution.
 */
export function inspectModule(bytes) {
  if (bytes.length < 8) throw new Error("not a WASM module (too short)");
  if (bytes.subarray(0, 4).toString("hex") !== "0061736d") {
    throw new Error("not a WASM module (missing \\0asm magic)");
  }
  if (bytes.readUInt32LE(4) !== 1) {
    throw new Error(`unsupported WASM binary version ${bytes.readUInt32LE(4)}`);
  }

  const types = [];
  const imports = [];
  const exports = [];
  const functionTypes = [];
  let importedFunctions = 0;

  let at = 8;
  while (at < bytes.length) {
    const id = bytes[at++];
    const [size, payloadStart] = readU32(bytes, at);
    const end = payloadStart + size;
    if (end > bytes.length) throw new Error("section extends past the end of the module");

    if (id === 1) {
      const [entries] = readVec(bytes, payloadStart, (b, i) => {
        if (b[i++] !== 0x60) throw new Error("type section entry is not a function type");
        const [params, afterParams] = readVec(b, i, (bb, ii) => [wasmType(bb[ii]), ii + 1]);
        const [results, afterResults] = readVec(b, afterParams, (bb, ii) => [wasmType(bb[ii]), ii + 1]);
        return [{ params, results }, afterResults];
      });
      types.push(...entries);
    } else if (id === 2) {
      const [entries] = readVec(bytes, payloadStart, (b, i) => {
        const [module, afterModule] = readName(b, i);
        const [name, afterName] = readName(b, afterModule);
        const kind = b[afterName];
        let cursor = afterName + 1;
        let signature = null;
        if (kind === 0x00) {
          const [typeIndex, afterType] = readU32(b, cursor);
          cursor = afterType;
          signature = typeIndex;
        } else if (kind === 0x01) {
          cursor += 1; // reftype
          const [flags, afterFlags] = readU32(b, cursor);
          cursor = afterFlags;
          const [min, afterMin] = readU32(b, cursor);
          cursor = afterMin;
          if (flags & 1) {
            const [, afterMax] = readU32(b, cursor);
            cursor = afterMax;
          }
          void min;
        } else if (kind === 0x02) {
          const [flags, afterFlags] = readU32(b, cursor);
          cursor = afterFlags;
          const [, afterMin] = readU32(b, cursor);
          cursor = afterMin;
          if (flags & 1) {
            const [, afterMax] = readU32(b, cursor);
            cursor = afterMax;
          }
        } else if (kind === 0x03) {
          cursor += 2; // valtype + mutability
        } else {
          throw new Error(`unknown import kind 0x${kind.toString(16)} for ${module}.${name}`);
        }
        if (kind === 0x00) importedFunctions += 1;
        return [{ module, name, kind, typeIndex: signature }, cursor];
      });
      imports.push(...entries);
    } else if (id === 3) {
      const [entries] = readVec(bytes, payloadStart, (b, i) => {
        const [typeIndex, after] = readU32(b, i);
        return [typeIndex, after];
      });
      functionTypes.push(...entries);
    } else if (id === 7) {
      const [entries] = readVec(bytes, payloadStart, (b, i) => {
        const [name, afterName] = readName(b, i);
        const kind = b[afterName];
        const [index, after] = readU32(b, afterName + 1);
        return [{ name, kind, index }, after];
      });
      exports.push(...entries);
    }
    at = end;
  }

  const signatureOf = (funcIndex) => {
    if (funcIndex < importedFunctions) {
      const imported = imports.filter((entry) => entry.kind === 0x00)[funcIndex];
      return types[imported.typeIndex];
    }
    const typeIndex = functionTypes[funcIndex - importedFunctions];
    return types[typeIndex];
  };

  return {
    imports: imports.map((entry) => ({
      module: entry.module,
      name: entry.name,
      kind: entry.kind,
      params: entry.kind === 0x00 ? (types[entry.typeIndex]?.params ?? []) : [],
      results: entry.kind === 0x00 ? (types[entry.typeIndex]?.results ?? []) : [],
    })),
    exports: exports.map((entry) => ({
      name: entry.name,
      kind: entry.kind,
      params: entry.kind === 0x00 ? (signatureOf(entry.index)?.params ?? []) : [],
      results: entry.kind === 0x00 ? (signatureOf(entry.index)?.results ?? []) : [],
    })),
  };
}

const expectedImportSignature = (fn) => ({
  params: fn.params.map(() => "i32"),
  results:
    fn.returns === "none" ? [] : fn.returns === "status" ? ["i32"] : ["i64"],
});

const exportSignature = (name) => {
  const found = REQUIRED_EXPORTS.find((entry) => entry.name === name);
  if (!found) return null;
  return {
    params: found.params,
    results: found.returns === null ? [] : [found.returns],
  };
};

const sameSignature = (a, b) =>
  a.params.length === b.params.length &&
  a.params.every((param, i) => param === b.params[i]) &&
  a.results.length === b.results.length &&
  a.results.every((result, i) => result === b.results[i]);

/**
 * Validates a `plugin.wasm` against the v1 ABI. Returns `{ ok, errors }`; never
 * throws for a contract violation (only for a module that is not parseable at
 * all, which the caller turns into the same message).
 */
export function validateModuleAbi(bytes) {
  const errors = [];
  let surface;
  try {
    surface = inspectModule(bytes);
  } catch (error) {
    return { ok: false, errors: [error.message] };
  }

  for (const imported of surface.imports) {
    if (imported.module !== ABI_NAMESPACE) {
      errors.push(
        `imports ${imported.module}.${imported.name}: the only import namespace is "${ABI_NAMESPACE}" (no WASI)`,
      );
      continue;
    }
    if (imported.kind !== 0x00) {
      errors.push(`imports ${ABI_NAMESPACE}.${imported.name}: only functions may be imported`);
      continue;
    }
    const declared = hostFunction(imported.name);
    if (!declared) {
      errors.push(
        `imports ${ABI_NAMESPACE}.${imported.name}: not one of the nine v1 host functions`,
      );
      continue;
    }
    const expected = expectedImportSignature(declared);
    if (!sameSignature(imported, expected)) {
      errors.push(
        `imports ${ABI_NAMESPACE}.${imported.name} as (${imported.params.join(", ")}) -> (${imported.results.join(", ")}); the ABI declares (${expected.params.join(", ")}) -> (${expected.results.join(", ")})`,
      );
    }
  }

  for (const required of REQUIRED_EXPORTS) {
    const found = surface.exports.find((entry) => entry.name === required.name);
    if (!found) {
      errors.push(`missing the required export ${required.name}`);
      continue;
    }
    if (found.kind !== 0x00) {
      errors.push(`${required.name} must be a function export`);
      continue;
    }
    const expected = exportSignature(required.name);
    if (!sameSignature(found, expected)) {
      errors.push(
        `${required.name} is (${found.params.join(", ")}) -> (${found.results.join(", ")}); the ABI declares (${expected.params.join(", ")}) -> (${expected.results.join(", ")})`,
      );
    }
  }

  return { ok: errors.length === 0, errors };
}
