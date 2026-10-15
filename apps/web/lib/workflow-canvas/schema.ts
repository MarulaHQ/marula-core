// The declared JSON-Schema subset, in TypeScript.
//
// Purpose: the schema-driven properties panel (ADR 0016 decision 5, spec C)
// renders a node's inputs from the registry's `input_schema` rather than a
// hand-written form per node — so this file owns the ONE client-side reading
// of that schema. It mirrors `crates/core-domain/src/node_registry.rs`'s
// closed subset exactly (`SUPPORTED_SCHEMA_KEYWORDS`) so a field that renders
// here is a field the server will accept, and the same keyword set is what
// `validateAgainstSchema` honours before the wire.
//
// Non-purpose: no JSON-Schema draft implementation. The subset is deliberately
// closed — a keyword this file cannot honour is a schema the server refuses at
// declaration time anyway, so we never silently ignore one.

export type JsonSchemaType =
  | "object"
  | "array"
  | "string"
  | "integer"
  | "number"
  | "boolean";

export interface JsonSchema {
  type?: JsonSchemaType;
  properties?: Record<string, JsonSchema>;
  required?: string[];
  additionalProperties?: boolean;
  items?: JsonSchema;
  minItems?: number;
  maxItems?: number;
  minLength?: number;
  maxLength?: number;
  enum?: unknown[];
  const?: unknown;
  format?: string;
  minimum?: number;
  maximum?: number;
  description?: string;
  title?: string;
}

export interface SchemaViolation {
  path: string;
  message: string;
}

/** A field the properties panel renders, derived from one schema property. */
export interface FieldDescriptor {
  key: string;
  label: string;
  description?: string;
  required: boolean;
  schema: JsonSchema;
  kind:
    | "string"
    | "text"
    | "number"
    | "integer"
    | "boolean"
    | "enum"
    | "const"
    | "uuid"
    | "array"
    | "object"
    | "unknown";
}

function isObj(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function kindOf(schema: JsonSchema): FieldDescriptor["kind"] {
  if (schema.const !== undefined) return "const";
  if (Array.isArray(schema.enum)) return "enum";
  switch (schema.type) {
    case "boolean":
      return "boolean";
    case "integer":
      return "integer";
    case "number":
      return "number";
    case "array":
      return "array";
    case "object":
      return "object";
    case "string":
      if (schema.format === "uuid") return "uuid";
      if (schema.maxLength !== undefined && schema.maxLength > 240) return "text";
      return "string";
    default:
      return "unknown";
  }
}

/**
 * Renders the object's fields in declaration order. A schema without
 * `properties` (e.g. a free-form object) yields no fields — the panel then
 * falls back to the raw params editor rather than pretending to know them.
 */
export function schemaFields(schema: JsonSchema | null | undefined): FieldDescriptor[] {
  if (!schema || schema.type !== "object" || !isObj(schema.properties)) return [];
  const required = new Set(Array.isArray(schema.required) ? schema.required : []);
  return Object.entries(schema.properties).map(([key, propSchema]) => {
    const prop = (isObj(propSchema) ? propSchema : {}) as JsonSchema;
    return {
      key,
      label: prop.title ?? humanise(key),
      description: prop.description,
      required: required.has(key),
      schema: prop,
      kind: kindOf(prop),
    };
  });
}

/** `boardId` → `Board id`; `maxItems` → `Max items`. Sentence case, no caps run. */
export function humanise(key: string): string {
  const spaced = key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/[_-]+/g, " ")
    .trim();
  return spaced.charAt(0).toUpperCase() + spaced.slice(1);
}

function violation(path: string, message: string): SchemaViolation {
  return { path, message };
}

function childPath(path: string, key: string): string {
  return path === "" ? key : `${path}.${key}`;
}

/**
 * Validates an instance against the declared subset, returning every
 * violation rather than the first, so the canvas can badge several fields at
 * once. Mirrors `node_registry::validate_against_schema`.
 */
export function validateAgainstSchema(
  schema: JsonSchema | null | undefined,
  instance: unknown,
  path = "",
): SchemaViolation[] {
  if (!schema) return [];
  const out: SchemaViolation[] = [];

  if (schema.const !== undefined && !deepEqual(schema.const, instance)) {
    out.push(violation(path, `must equal ${JSON.stringify(schema.const)}`));
    return out;
  }
  if (Array.isArray(schema.enum) && !schema.enum.some((v) => deepEqual(v, instance))) {
    out.push(violation(path, `must be one of ${schema.enum.map((v) => JSON.stringify(v)).join(", ")}`));
    return out;
  }

  switch (schema.type) {
    case "object": {
      if (!isObj(instance)) {
        out.push(violation(path, "must be an object"));
        break;
      }
      const props = isObj(schema.properties) ? schema.properties : {};
      for (const key of schema.required ?? []) {
        if (!(key in instance) || instance[key] === undefined) {
          out.push(violation(childPath(path, key), "is required"));
        }
      }
      if (schema.additionalProperties === false) {
        for (const key of Object.keys(instance)) {
          if (!(key in props)) out.push(violation(childPath(path, key), "is not a known field"));
        }
      }
      for (const [key, propSchema] of Object.entries(props)) {
        if (key in instance && instance[key] !== undefined) {
          out.push(
            ...validateAgainstSchema(
              propSchema as JsonSchema,
              instance[key],
              childPath(path, key),
            ),
          );
        }
      }
      break;
    }
    case "array": {
      if (!Array.isArray(instance)) {
        out.push(violation(path, "must be an array"));
        break;
      }
      if (schema.minItems !== undefined && instance.length < schema.minItems) {
        out.push(violation(path, `needs at least ${schema.minItems} item(s)`));
      }
      if (schema.maxItems !== undefined && instance.length > schema.maxItems) {
        out.push(violation(path, `allows at most ${schema.maxItems} item(s)`));
      }
      if (schema.items) {
        instance.forEach((item, i) => {
          out.push(...validateAgainstSchema(schema.items, item, `${path}[${i}]`));
        });
      }
      break;
    }
    case "string": {
      if (typeof instance !== "string") {
        out.push(violation(path, "must be text"));
        break;
      }
      if (schema.minLength !== undefined && instance.length < schema.minLength) {
        out.push(violation(path, `needs at least ${schema.minLength} characters`));
      }
      if (schema.maxLength !== undefined && instance.length > schema.maxLength) {
        out.push(violation(path, `allows at most ${schema.maxLength} characters`));
      }
      if (schema.format === "uuid" && !UUID_RE.test(instance)) {
        out.push(violation(path, "must be a UUID"));
      }
      break;
    }
    case "integer": {
      if (typeof instance !== "number" || !Number.isInteger(instance)) {
        out.push(violation(path, "must be a whole number"));
        break;
      }
      out.push(...numberBounds(schema, instance, path));
      break;
    }
    case "number": {
      if (typeof instance !== "number" || Number.isNaN(instance)) {
        out.push(violation(path, "must be a number"));
        break;
      }
      out.push(...numberBounds(schema, instance, path));
      break;
    }
    case "boolean": {
      if (typeof instance !== "boolean") out.push(violation(path, "must be true or false"));
      break;
    }
    default:
      break;
  }
  return out;
}

function numberBounds(schema: JsonSchema, value: number, path: string): SchemaViolation[] {
  const out: SchemaViolation[] = [];
  if (schema.minimum !== undefined && value < schema.minimum) {
    out.push(violation(path, `must be at least ${schema.minimum}`));
  }
  if (schema.maximum !== undefined && value > schema.maximum) {
    out.push(violation(path, `must be at most ${schema.maximum}`));
  }
  return out;
}

function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) && Array.isArray(b)) {
    return a.length === b.length && a.every((v, i) => deepEqual(v, b[i]));
  }
  if (isObj(a) && isObj(b)) {
    const ak = Object.keys(a);
    const bk = Object.keys(b);
    return ak.length === bk.length && ak.every((k) => deepEqual(a[k], b[k]));
  }
  return false;
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Seeds a params object from a schema's `enum`/`const` defaults so a freshly
 * dropped node is immediately valid where the catalogue offers a first value
 * (e.g. a trigger moment, a branch condition). Never invents a value for a
 * required field with no candidate — the panel surfaces it as missing instead.
 */
export function seedParams(schema: JsonSchema | null | undefined): Record<string, unknown> {
  const params: Record<string, unknown> = {};
  if (!schema || schema.type !== "object" || !isObj(schema.properties)) return params;
  for (const [key, raw] of Object.entries(schema.properties)) {
    const prop = raw as JsonSchema;
    if (prop.const !== undefined) params[key] = prop.const;
    else if (Array.isArray(prop.enum) && prop.enum.length === 1) params[key] = prop.enum[0];
  }
  return params;
}
