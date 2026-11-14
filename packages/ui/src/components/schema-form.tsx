"use client";
/**
 * SchemaForm — renders a connector manifest's `settings_schema` as a form.
 *
 * Purpose: a connector contributes declarations, never layout, so the one form
 * Core owns has to render from the pinned manifest. This component is that
 * form. It honours exactly the subset `core_domain::entities::SettingsSchema`
 * accepts — see `docs/reference/connector-specification.md` §3.1 — because a
 * renderer that draws a keyword the write route ignores, or omits one it
 * requires, publishes a form the API refuses.
 *
 * The subset, and what each keyword means here:
 *
 * - `type` — `string`, `integer`, `number`, `boolean`, `object`, `array`.
 * - `properties` + `required` — an object's fields. Nested objects recurse;
 *   nesting deeper than four levels is refused by the server, so a schema that
 *   deep is refused here too and the caller draws the typed reason.
 * - `items` + `minItems` / `maxItems` — array element schema and length bounds.
 * - `enum` — a closed set of scalars. The `<Select>` carries the index, never
 *   the display text, so a numeric enum keeps its number.
 * - `minimum` / `maximum` — numeric bounds.
 * - `minLength` / `maxLength` — string length in characters (code points, as
 *   the server counts them).
 * - `pattern` — an unanchored regular expression. Matching is unanchored here
 *   for the same reason: a declaration that wants the whole value anchors it.
 * - `writeOnly: true` — a credential. Top level only, because the server's
 *   `write_only_keys()` is top level only. It renders as a masked field, is
 *   never initialised from a stored value, and is reported through
 *   `onSecretsChange` so the request can put it in `secrets` rather than
 *   `settings`. A credential sent in `settings` is refused by the server.
 *
 * Annotations the subset does not interpret (`title`, `description`, `format`)
 * are read for labels and hints and never validated against. Unknown keys are
 * not renderable and are not invented: a value naming a key the schema does
 * not declare is refused by the same validator the settings object goes
 * through (`validateSettingsValue`), which mirrors the server's deny-by-default
 * rule at every object level.
 *
 * Design research (lazyweb `lazyweb_search_screens`, 2026-11-14): the Auth0
 * "create user" modal and Tray's auth drawer both put a short labelled field
 * list on one column with one primary action; Okta's app picker and Knock's
 * provider picker show a closed set as rows, one selected. Both readings land
 * here: labels above fields, one column, no wizard, and closed sets as a
 * select rather than free text. Palette, type, spacing and motion stay
 * `docs/theme.md`'s — the references supply composition only.
 */
import { useId, useState, type ReactNode } from "react";
import { Button } from "./button";
import { Input } from "./input";
import { Label, FieldError, FieldHint } from "./field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./select";
import { Switch } from "../kit";

/** One declared schema node. Indexed because annotations ride along. */
export interface SettingsSchemaNode {
  type?: string;
  properties?: Record<string, SettingsSchemaNode>;
  required?: string[];
  items?: SettingsSchemaNode;
  enum?: SettingsScalar[];
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  minItems?: number;
  maxItems?: number;
  writeOnly?: boolean;
  title?: string;
  description?: string;
  format?: string;
  default?: SettingsScalar;
}

export type SettingsScalar = string | number | boolean;

export type SettingsValue =
  | SettingsScalar
  | null
  | SettingsValue[]
  | { [key: string]: SettingsValue };

/** A settings document's non-credential half. */
export type SettingsObject = Record<string, SettingsValue>;

/** Credential drafts, keyed by the schema's top-level `writeOnly` field. */
export type SecretDrafts = Record<string, string>;

/** Field errors keyed by dotted / bracketed path (`network.port`, `hosts[0]`). */
export type SettingsErrors = Record<string, string>;

/** The deepest `object`/`array` nesting the server accepts. */
export const SETTINGS_SCHEMA_MAX_DEPTH = 4;

/**
 * Constructs `SettingsSchema::new` refuses at publish time. A manifest that
 * declares one is refused by the server, so the form refuses to render it
 * rather than guessing what it meant.
 */
export const SETTINGS_SCHEMA_REFUSED_KEYWORDS = [
  "oneOf",
  "anyOf",
  "allOf",
  "not",
  "$ref",
  "additionalProperties",
  "patternProperties",
  "propertyNames",
  "multipleOf",
  "exclusiveMinimum",
  "exclusiveMaximum",
  "uniqueItems",
  "contains",
  "dependencies",
  "dependentRequired",
  "if",
  "then",
  "else",
] as const;

const SUPPORTED_TYPES = new Set([
  "string",
  "number",
  "integer",
  "boolean",
  "object",
  "array",
]);

export type SchemaReadResult =
  | { ok: true; schema: SettingsSchemaNode }
  | { ok: false; reason: string };

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The top-level `writeOnly` names — the credential fields, and only those. */
export function writeOnlyKeys(schema: SettingsSchemaNode | null | undefined): string[] {
  const props = schema?.properties;
  if (props === undefined) return [];
  return Object.entries(props)
    .filter(([, spec]) => spec.writeOnly === true)
    .map(([key]) => key);
}

/** Humanised fallback label: `api_base_url` → "Api base url". */
export function fieldLabel(name: string, node: SettingsSchemaNode): string {
  if (typeof node.title === "string" && node.title.trim() !== "") return node.title;
  const words = name.replace(/[_-]+/g, " ").trim();
  if (words === "") return name;
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function declaredNodeRejection(
  node: Record<string, unknown>,
  path: string,
  depth: number,
): string | null {
  const where = path === "" ? "the settings schema" : path;
  if (depth > SETTINGS_SCHEMA_MAX_DEPTH) {
    return `${where} nests deeper than ${String(SETTINGS_SCHEMA_MAX_DEPTH)} levels, which the server refuses.`;
  }
  for (const keyword of SETTINGS_SCHEMA_REFUSED_KEYWORDS) {
    if (keyword in node) {
      return `${where} declares "${keyword}", which this schema subset does not honour.`;
    }
  }
  const type = node.type;
  if (typeof type !== "string") return `${where} declares no "type".`;
  if (!SUPPORTED_TYPES.has(type)) {
    return `${where} declares unsupported type "${type}".`;
  }
  if (Array.isArray(node.enum)) {
    if (node.enum.length === 0) return `${where} has an empty enum.`;
    for (const item of node.enum) {
      if (item === null || typeof item === "object") {
        return `${where} has an enum of non-scalar values.`;
      }
    }
  } else if (node.enum !== undefined) {
    return `${where} has an enum that is not an array.`;
  }

  if (type === "object") {
    const props = node.properties;
    if (!isPlainObject(props)) {
      // The top level may omit `properties` ("this connector has no
      // settings"); a nested object may not, because it would accept any key.
      if (depth === 0) return null;
      return `${where} is an object with no "properties".`;
    }
    const required = node.required;
    if (required !== undefined) {
      if (!Array.isArray(required)) return `${where} has a "required" that is not an array.`;
      for (const name of required) {
        if (typeof name !== "string") return `${where} has a non-string entry in "required".`;
        if (!(name in props)) {
          return `${where} requires "${name}" without declaring it.`;
        }
      }
    }
    for (const [key, child] of Object.entries(props)) {
      const childPath = path === "" ? key : `${path}.${key}`;
      if (!isPlainObject(child)) return `${childPath} is not a JSON object.`;
      const childRejection = declaredNodeRejection(child, childPath, depth + 1);
      if (childRejection !== null) return childRejection;
    }
  }
  if (type === "array") {
    if (!isPlainObject(node.items)) return `${where} is an array with no "items" schema.`;
    const itemRejection = declaredNodeRejection(node.items, `${path}[]`, depth + 1);
    if (itemRejection !== null) return itemRejection;
  }

  const minLength = node.minLength;
  const maxLength = node.maxLength;
  if (typeof minLength === "number" && typeof maxLength === "number" && maxLength < minLength) {
    return `${where} has maxLength < minLength.`;
  }
  const minimum = node.minimum;
  const maximum = node.maximum;
  if (typeof minimum === "number" && typeof maximum === "number" && maximum < minimum) {
    return `${where} has maximum < minimum.`;
  }
  const minItems = node.minItems;
  const maxItems = node.maxItems;
  if (typeof minItems === "number" && typeof maxItems === "number" && maxItems < minItems) {
    return `${where} has maxItems < minItems.`;
  }
  return null;
}

/**
 * Reads a raw manifest `settings_schema` into a renderable node, or names why
 * it cannot be rendered. Refusal is the honest answer: a schema outside the
 * subset is a manifest the API would refuse, and drawing half of it would let
 * an operator fill in a form that can never be saved.
 */
export function readSettingsSchema(raw: unknown): SchemaReadResult {
  if (raw === null || raw === undefined) {
    return { ok: false, reason: "this connector declares no settings schema" };
  }
  if (!isPlainObject(raw)) {
    return { ok: false, reason: "the settings schema is not a JSON object" };
  }
  if (raw.type !== "object") {
    return { ok: false, reason: 'the settings schema must declare "type": "object"' };
  }
  const rejection = declaredNodeRejection(raw, "", 0);
  if (rejection !== null) return { ok: false, reason: rejection };
  return { ok: true, schema: raw as SettingsSchemaNode };
}

function joinPath(path: string, key: string): string {
  return path === "" ? key : `${path}.${key}`;
}

function patternMatches(pattern: string, value: string): boolean | null {
  try {
    // Unanchored, matching the server's `is_match`. A pattern Rust accepts and
    // JavaScript does not is unverifiable here, and `null` says so: the client
    // check is never the only check, so the server still judges the value.
    return new RegExp(pattern).test(value);
  } catch {
    return null;
  }
}

function validateNode(
  node: SettingsSchemaNode,
  value: SettingsValue | undefined,
  path: string,
  errors: SettingsErrors,
): void {
  const label = path === "" ? "settings" : `settings key "${path}"`;
  if (Array.isArray(node.enum)) {
    if (!node.enum.some((candidate) => candidate === value)) {
      errors[path] = `${label} is not one of the declared values.`;
    }
    return;
  }
  switch (node.type) {
    case "string": {
      if (typeof value !== "string") {
        errors[path] = `${label} must be text.`;
        return;
      }
      const length = Array.from(value).length;
      if (node.minLength !== undefined && length < node.minLength) {
        errors[path] = `${label} is shorter than ${String(node.minLength)} characters.`;
        return;
      }
      if (node.maxLength !== undefined && length > node.maxLength) {
        errors[path] = `${label} is longer than ${String(node.maxLength)} characters.`;
        return;
      }
      if (node.pattern !== undefined) {
        const matched = patternMatches(node.pattern, value);
        if (matched === false) errors[path] = `${label} does not match its declared pattern.`;
      }
      return;
    }
    case "number": {
      if (typeof value !== "number" || !Number.isFinite(value)) {
        errors[path] = `${label} must be a number.`;
        return;
      }
      if (node.minimum !== undefined && value < node.minimum) {
        errors[path] = `${label} is below its declared minimum of ${String(node.minimum)}.`;
        return;
      }
      if (node.maximum !== undefined && value > node.maximum) {
        errors[path] = `${label} is above its declared maximum of ${String(node.maximum)}.`;
      }
      return;
    }
    case "integer": {
      if (typeof value !== "number" || !Number.isInteger(value)) {
        errors[path] = `${label} must be a whole number.`;
        return;
      }
      if (node.minimum !== undefined && value < node.minimum) {
        errors[path] = `${label} is below its declared minimum of ${String(node.minimum)}.`;
        return;
      }
      if (node.maximum !== undefined && value > node.maximum) {
        errors[path] = `${label} is above its declared maximum of ${String(node.maximum)}.`;
      }
      return;
    }
    case "boolean": {
      if (typeof value !== "boolean") errors[path] = `${label} must be true or false.`;
      return;
    }
    case "object": {
      if (!isPlainObject(value)) {
        errors[path] = `${label} must be an object.`;
        return;
      }
      const props = node.properties ?? {};
      for (const key of Object.keys(value)) {
        if (!(key in props)) {
          const keyPath = joinPath(path, key);
          errors[keyPath] = `settings key "${keyPath}" is not declared by this connector's schema.`;
        }
      }
      for (const name of node.required ?? []) {
        if (!(name in value) || value[name] === null || value[name] === undefined) {
          const keyPath = joinPath(path, name);
          errors[keyPath] = `settings key "${keyPath}" is required.`;
        }
      }
      for (const [key, child] of Object.entries(props)) {
        if (!(key in value)) continue;
        validateNode(child, value[key], joinPath(path, key), errors);
      }
      return;
    }
    case "array": {
      if (!Array.isArray(value)) {
        errors[path] = `${label} must be a list.`;
        return;
      }
      if (node.minItems !== undefined && value.length < node.minItems) {
        errors[path] = `${label} needs at least ${String(node.minItems)} item${node.minItems === 1 ? "" : "s"}.`;
        return;
      }
      if (node.maxItems !== undefined && value.length > node.maxItems) {
        errors[path] = `${label} allows at most ${String(node.maxItems)} item${node.maxItems === 1 ? "" : "s"}.`;
        return;
      }
      if (node.items !== undefined) {
        value.forEach((item, index) => {
          validateNode(node.items ?? {}, item, `${path}[${String(index)}]`, errors);
        });
      }
      return;
    }
    default:
      errors[path] = `${label} declares a type this form cannot render (${String(node.type)}).`;
  }
}

/**
 * The client half of the write boundary. Mirrors `SettingsSchema::validate_config`
 * and the split rule in `validate_settings`:
 *
 * - an unknown key is refused at every object level;
 * - a `writeOnly` key in `settings` is refused, and a `secrets` key the schema
 *   does not mark `writeOnly` is refused;
 * - required credential fields are satisfied by a draft or by an already
 *   configured value (`configuredSecrets`), never by a value in `settings`.
 *
 * The server is still the only authority: this returns a map for the form to
 * render, and a caller that skips it still gets a 422.
 */
export function validateSettingsValue(
  schema: SettingsSchemaNode,
  settings: SettingsObject,
  secrets: SecretDrafts = {},
  configuredSecrets: readonly string[] = [],
): SettingsErrors {
  const errors: SettingsErrors = {};
  const credentialNames = new Set(writeOnlyKeys(schema));

  for (const key of Object.keys(settings)) {
    if (credentialNames.has(key) && settings[key] !== null && settings[key] !== undefined) {
      errors[key] = `settings key "${key}" is a credential — it is sent separately, never in settings.`;
    }
  }
  for (const key of Object.keys(secrets)) {
    if (!credentialNames.has(key)) {
      errors[key] = `"${key}" is not a credential field of this connector — send it in settings.`;
    }
  }

  const settingsWithoutCredentials: SettingsObject = {};
  for (const [key, value] of Object.entries(settings)) {
    if (credentialNames.has(key)) continue;
    settingsWithoutCredentials[key] = value;
  }
  validateNode(schema, settingsWithoutCredentials, "", errors);

  const configured = new Set(configuredSecrets);
  const requiredNames = schema.required ?? [];
  for (const key of credentialNames) {
    const draft = secrets[key];
    const hasDraft = typeof draft === "string" && draft !== "";
    const isConfigured = configured.has(key);
    if (hasDraft && !isConfigured) {
      // The credential is a value like any other; judge it against its own
      // declaration so a numeric credential with text is refused here too.
      const node = schema.properties?.[key];
      if (node !== undefined) {
        const asValue: SettingsValue =
          node.type === "number" || node.type === "integer"
            ? Number(draft)
            : node.type === "boolean"
              ? draft === "true"
              : draft;
        validateNode(node, asValue, key, errors);
      }
    }
    if (!hasDraft && !isConfigured) {
      if (requiredNames.includes(key) && errors[key] === undefined) {
        errors[key] = `settings key "${key}" is required.`;
      }
      continue;
    }
    // A satisfied credential is present in the server's merged document, so
    // the "required" complaint validateNode raised against settings alone is
    // not true.
    if (errors[key]?.endsWith("is required.") === true) delete errors[key];
  }
  return errors;
}

/** Reads one form value as the string a controlled input needs. */
function textOf(value: SettingsValue | undefined): string {
  if (typeof value === "string") return value;
  if (typeof value === "number") return String(value);
  return "";
}

function numberFromText(text: string): number | string | undefined {
  if (text.trim() === "") return undefined;
  const parsed = Number(text);
  return Number.isFinite(parsed) ? parsed : text;
}

function cloneAtPath(
  value: SettingsObject,
  segments: readonly string[],
  next: SettingsValue | undefined,
): SettingsObject {
  const [head, ...rest] = segments;
  if (head === undefined) return value;
  const copy: SettingsObject = { ...value };
  if (rest.length === 0) {
    if (next === undefined) delete copy[head];
    else copy[head] = next;
    return copy;
  }
  const child = copy[head];
  const childObject = isPlainObject(child) ? (child as SettingsObject) : {};
  copy[head] = cloneAtPath(childObject, rest, next);
  return copy;
}

function hintFor(node: SettingsSchemaNode): string | null {
  if (typeof node.description === "string" && node.description.trim() !== "") {
    return node.description;
  }
  if (Array.isArray(node.enum)) return "One of the declared values.";
  if (node.type === "array") {
    const bounds: string[] = [];
    if (node.minItems !== undefined) bounds.push(`at least ${String(node.minItems)}`);
    if (node.maxItems !== undefined) bounds.push(`at most ${String(node.maxItems)}`);
    return bounds.length === 0 ? null : `A list, ${bounds.join(" and ")}.`;
  }
  if (node.type === "string") {
    const bounds: string[] = [];
    if (node.minLength !== undefined) bounds.push(`at least ${String(node.minLength)} characters`);
    if (node.maxLength !== undefined) bounds.push(`at most ${String(node.maxLength)} characters`);
    if (node.pattern !== undefined) bounds.push("must match the connector's declared pattern");
    return bounds.length === 0 ? null : `Text, ${bounds.join(", ")}.`;
  }
  if (node.type === "number" || node.type === "integer") {
    const bounds: string[] = [];
    if (node.minimum !== undefined) bounds.push(`minimum ${String(node.minimum)}`);
    if (node.maximum !== undefined) bounds.push(`maximum ${String(node.maximum)}`);
    if (bounds.length === 0) return node.type === "integer" ? "A whole number." : null;
    return `A ${node.type === "integer" ? "whole number" : "number"}, ${bounds.join(", ")}.`;
  }
  return null;
}

function slugFor(path: string): string {
  return path.replace(/[^a-zA-Z0-9_-]+/g, "-");
}

function fieldIds(
  path: string,
  idPrefix: string,
  hasError: boolean,
  hasHint: boolean,
): {
  controlId: string;
  labelId: string;
  hintId: string;
  errorId: string;
  /** Only the ids that are actually rendered, so the attribute never dangles. */
  describedBy: string | undefined;
} {
  const controlId = `${idPrefix}-${slugFor(path)}`;
  const hintId = `${controlId}-hint`;
  const errorId = `${controlId}-error`;
  const described = [
    ...(hasHint ? [hintId] : []),
    ...(hasError ? [errorId] : []),
  ];
  return {
    controlId,
    labelId: `${controlId}-label`,
    hintId,
    errorId,
    describedBy: described.length === 0 ? undefined : described.join(" "),
  };
}

export interface SchemaFormProps {
  schema: SettingsSchemaNode;
  /** The settings document, without credentials. */
  value: SettingsObject;
  onChange: (next: SettingsObject) => void;
  /** Credential fields the server reports as holding a value. Never values. */
  secretsSet?: readonly string[];
  /** Replacement credential values. Always starts empty; never pre-filled. */
  secrets?: SecretDrafts;
  onSecretsChange?: (next: SecretDrafts) => void;
  /** Server-side errors, keyed by path. Rendered alongside client errors. */
  serverErrors?: SettingsErrors;
  disabled?: boolean;
  idPrefix?: string;
}

/**
 * Renders the schema. Every control is labelled, every error is associated via
 * `aria-describedby`, and validation runs on blur as well as on submit.
 */
export function SchemaForm({
  schema,
  value,
  onChange,
  secretsSet = [],
  secrets = {},
  onSecretsChange,
  serverErrors = {},
  disabled = false,
  idPrefix,
}: SchemaFormProps): ReactNode {
  const generatedId = useId();
  const prefix = idPrefix ?? `schema-${generatedId.replace(/[^a-zA-Z0-9_-]+/g, "")}`;
  const [blurred, setBlurred] = useState<ReadonlySet<string>>(new Set());

  const properties = schema.properties ?? {};
  const names = Object.keys(properties);
  const credentialNames = new Set(writeOnlyKeys(schema));
  const configured = new Set(secretsSet);

  if (names.length === 0) {
    return (
      <p style={{ margin: 0, fontSize: 13, color: "var(--m-text-secondary)" }}>
        This connector declares no settings. The credential fields, if any, are the whole
        configuration.
      </p>
    );
  }

  const blur = (path: string): void => {
    setBlurred((previous) => {
      const next = new Set(previous);
      next.add(path);
      return next;
    });
  };

  const clientErrorAt = (
    path: string,
    node: SettingsSchemaNode,
    current: SettingsValue | undefined,
  ): string | undefined => {
    const local: SettingsErrors = {};
    validateNode(node, current, path, local);
    return local[path] ?? Object.values(local)[0];
  };

  const renderField = (
    name: string,
    node: SettingsSchemaNode,
    path: string,
    current: SettingsValue | undefined,
    depth: number,
  ): ReactNode => {
    const isCredential = depth === 0 && credentialNames.has(name);
    const hint = hintFor(node);
    const clientError = blurred.has(path)
      ? clientErrorAt(path, node, isCredential ? undefined : current)
      : undefined;
    const error = serverErrors[path] ?? clientError;
    const ids = fieldIds(path, prefix, error !== undefined, hint !== null);

    if (isCredential) {
      const isConfigured = configured.has(name);
      return (
        <div key={path} style={{ display: "grid", gap: 4 }} data-field-path={path}>
          <Label id={ids.labelId} htmlFor={ids.controlId}>
            {fieldLabel(name, node)}
            <span style={{ marginLeft: 6, fontWeight: 400, color: "var(--m-text-muted)" }}>
              credential
            </span>
          </Label>
          <Input
            id={ids.controlId}
            type="password"
            autoComplete="new-password"
            value={secrets[name] ?? ""}
            placeholder={isConfigured ? "Configured — type to replace" : "Not configured yet"}
            aria-describedby={ids.describedBy}
            aria-invalid={error !== undefined ? true : undefined}
            disabled={disabled}
            onBlur={() => {
              blur(path);
            }}
            onChange={(event) => {
              const next = { ...secrets };
              if (event.target.value === "") delete next[name];
              else next[name] = event.target.value;
              onSecretsChange?.(next);
            }}
          />
          <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>
            {isConfigured
              ? "A value is stored. It is never shown again; leave this blank to keep it."
              : "Stored sealed. The API never returns it."}
          </p>
          {hint !== null ? <FieldHint id={ids.hintId}>{hint}</FieldHint> : null}
          {error !== undefined ? <FieldError id={ids.errorId}>{error}</FieldError> : null}
        </div>
      );
    }

    if (Array.isArray(node.enum)) {
      const enumValues = node.enum;
      const selectedIndex = enumValues.findIndex((candidate) => candidate === current);
      return (
        <div key={path} style={{ display: "grid", gap: 4 }} data-field-path={path}>
          <Label id={ids.labelId} htmlFor={ids.controlId}>
            {fieldLabel(name, node)}
          </Label>
          <Select
            value={selectedIndex >= 0 ? String(selectedIndex) : undefined}
            disabled={disabled}
            onValueChange={(next) => {
              const candidate = enumValues[Number(next)];
              if (candidate !== undefined) onChange(cloneAtPath(value, path.split("."), candidate));
              blur(path);
            }}
          >
            <SelectTrigger
              id={ids.controlId}
              aria-labelledby={ids.labelId}
              aria-describedby={ids.describedBy}
              aria-invalid={error !== undefined ? true : undefined}
            >
              <SelectValue placeholder="Choose one" />
            </SelectTrigger>
            <SelectContent>
              {enumValues.map((candidate, index) => (
                <SelectItem key={String(index)} value={String(index)}>
                  {String(candidate)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {hint !== null ? <FieldHint id={ids.hintId}>{hint}</FieldHint> : null}
          {error !== undefined ? <FieldError id={ids.errorId}>{error}</FieldError> : null}
        </div>
      );
    }

    switch (node.type) {
      case "boolean":
        return (
          <div key={path} style={{ display: "grid", gap: 4 }} data-field-path={path}>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <Switch
                id={ids.controlId}
                checked={current === true}
                aria-labelledby={ids.labelId}
                aria-describedby={ids.describedBy}
                disabled={disabled}
                onCheckedChange={(checked) => {
                  onChange(cloneAtPath(value, path.split("."), checked));
                  blur(path);
                }}
              />
              <Label id={ids.labelId} htmlFor={ids.controlId}>
                {fieldLabel(name, node)}
              </Label>
            </div>
            {hint !== null ? <FieldHint id={ids.hintId}>{hint}</FieldHint> : null}
            {error !== undefined ? <FieldError id={ids.errorId}>{error}</FieldError> : null}
          </div>
        );
      case "integer":
      case "number":
        return (
          <div key={path} style={{ display: "grid", gap: 4 }} data-field-path={path}>
            <Label id={ids.labelId} htmlFor={ids.controlId}>
              {fieldLabel(name, node)}
            </Label>
            <Input
              id={ids.controlId}
              type="number"
              inputMode={node.type === "integer" ? "numeric" : "decimal"}
              step={node.type === "integer" ? 1 : "any"}
              min={node.minimum}
              max={node.maximum}
              value={textOf(current)}
              aria-describedby={ids.describedBy}
              aria-invalid={error !== undefined ? true : undefined}
              disabled={disabled}
              onBlur={() => {
                blur(path);
              }}
              onChange={(event) => {
                onChange(cloneAtPath(value, path.split("."), numberFromText(event.target.value)));
              }}
            />
            {hint !== null ? <FieldHint id={ids.hintId}>{hint}</FieldHint> : null}
            {error !== undefined ? <FieldError id={ids.errorId}>{error}</FieldError> : null}
          </div>
        );
      case "array":
        return (
          <ArrayField
            key={path}
            name={name}
            node={node}
            path={path}
            items={Array.isArray(current) ? current : []}
            ids={ids}
            hint={hint}
            error={error}
            disabled={disabled}
            onBlur={() => {
              blur(path);
            }}
            onChange={(next) => {
              onChange(cloneAtPath(value, path.split("."), next));
            }}
            renderItem={(item, index) => (
              <ArrayItem
                item={item}
                index={index}
                node={node.items ?? {}}
                path={`${path}[${String(index)}]`}
                prefix={prefix}
                label={fieldLabel(name, node)}
                disabled={disabled}
                onBlur={blur}
                onChange={(next) => {
                  const copy = [...itemsOf(current)];
                  copy[index] = next;
                  onChange(cloneAtPath(value, path.split("."), copy));
                }}
              />
            )}
            renderObject={(objectValue, objectPath) => (
              <ObjectFields
                node={node.items ?? {}}
                current={objectValue}
                path={objectPath}
                renderChild={(childName, childNode, childPath, childValue) =>
                  renderField(childName, childNode, childPath, childValue, depth + 2)
                }
              />
            )}
          />
        );
      case "object":
        return (
          <fieldset
            key={path}
            style={{
              display: "grid",
              gap: 10,
              margin: 0,
              padding: 10,
              border: "1px solid var(--m-border)",
              borderRadius: "var(--m-radius-control)",
            }}
            data-field-path={path}
          >
            <legend style={{ padding: "0 4px", fontSize: 12, color: "var(--m-text-secondary)" }}>
              {fieldLabel(name, node)}
            </legend>
            {hint !== null ? <FieldHint id={ids.hintId}>{hint}</FieldHint> : null}
            {error !== undefined ? <FieldError id={ids.errorId}>{error}</FieldError> : null}
            <ObjectFields
              node={node}
              current={isPlainObject(current) ? (current as SettingsObject) : {}}
              path={path}
              renderChild={(childName, childNode, childPath, childValue) =>
                renderField(childName, childNode, childPath, childValue, depth + 1)
              }
            />
          </fieldset>
        );
      case "string":
      default:
        return (
          <div key={path} style={{ display: "grid", gap: 4 }} data-field-path={path}>
            <Label id={ids.labelId} htmlFor={ids.controlId}>
              {fieldLabel(name, node)}
            </Label>
            <Input
              id={ids.controlId}
              type="text"
              value={textOf(current)}
              aria-describedby={ids.describedBy}
              aria-invalid={error !== undefined ? true : undefined}
              disabled={disabled}
              onBlur={() => {
                blur(path);
              }}
              onChange={(event) => {
                onChange(cloneAtPath(value, path.split("."), event.target.value));
              }}
            />
            {hint !== null ? <FieldHint id={ids.hintId}>{hint}</FieldHint> : null}
            {error !== undefined ? <FieldError id={ids.errorId}>{error}</FieldError> : null}
          </div>
        );
    }
  };

  return (
    <div style={{ display: "grid", gap: 12 }} data-testid="schema-form">
      {names.map((name) => {
        const node = properties[name];
        if (node === undefined) return null;
        return renderField(name, node, name, value[name], 0);
      })}
    </div>
  );
}

function itemsOf(value: SettingsValue | undefined): SettingsValue[] {
  return Array.isArray(value) ? value : [];
}

function ObjectFields({
  node,
  current,
  path,
  renderChild,
}: {
  node: SettingsSchemaNode;
  current: SettingsObject;
  path: string;
  renderChild: (
    name: string,
    node: SettingsSchemaNode,
    path: string,
    value: SettingsValue | undefined,
  ) => ReactNode;
}): ReactNode {
  const properties = node.properties ?? {};
  const names = Object.keys(properties);
  if (names.length === 0) {
    return (
      <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>
        This object declares no fields.
      </p>
    );
  }
  return (
    <>
      {names.map((name) => {
        const child = properties[name];
        if (child === undefined) return null;
        const childPath = path === "" ? name : `${path}.${name}`;
        return renderChild(name, child, childPath, current[name]);
      })}
    </>
  );
}

function ArrayField({
  name,
  node,
  path,
  items,
  ids,
  hint,
  error,
  disabled,
  onBlur,
  onChange,
  renderItem,
  renderObject,
}: {
  name: string;
  node: SettingsSchemaNode;
  path: string;
  items: SettingsValue[];
  ids: ReturnType<typeof fieldIds>;
  hint: string | null;
  error: string | undefined;
  disabled: boolean;
  onBlur: () => void;
  onChange: (next: SettingsValue[]) => void;
  renderItem: (item: SettingsValue, index: number) => ReactNode;
  renderObject: (value: SettingsObject, path: string) => ReactNode;
}): ReactNode {
  const itemNode = node.items ?? {};
  const isObjectItem = itemNode.type === "object";
  const atMinimum = node.minItems !== undefined && items.length <= node.minItems;
  const atMaximum = node.maxItems !== undefined && items.length >= node.maxItems;
  return (
    <fieldset
      style={{
        display: "grid",
        gap: 8,
        margin: 0,
        padding: 10,
        border: "1px solid var(--m-border)",
        borderRadius: "var(--m-radius-control)",
      }}
      data-field-path={path}
    >
      <legend style={{ padding: "0 4px", fontSize: 12, color: "var(--m-text-secondary)" }}>
        {fieldLabel(name, node)}
      </legend>
      {items.length === 0 ? (
        <p style={{ margin: 0, fontSize: 12, color: "var(--m-text-muted)" }}>No items yet.</p>
      ) : null}
      {items.map((item, index) => {
        const itemPath = `${path}[${String(index)}]`;
        return (
          <div key={itemPath} style={{ display: "flex", alignItems: "flex-start", gap: 8 }}>
            <div style={{ flex: 1, minWidth: 0 }}>
              {isObjectItem
                ? renderObject(
                    isPlainObject(item) ? (item as SettingsObject) : {},
                    itemPath,
                  )
                : renderItem(item, index)}
            </div>
            <Button
              size="sm"
              variant="ghost"
              disabled={disabled || atMinimum}
              aria-label={`Remove item ${String(index + 1)}`}
              onClick={() => {
                onChange(items.filter((_, position) => position !== index));
              }}
            >
              Remove
            </Button>
          </div>
        );
      })}
      <div style={{ display: "grid", gap: 2, justifyItems: "start" }}>
        <Button
          size="sm"
          variant="secondary"
          disabled={disabled || atMaximum}
          onBlur={onBlur}
          onClick={() => {
            onChange([...items, blankFor(itemNode)]);
          }}
        >
          Add item
        </Button>
        {atMaximum ? (
          <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
            This list allows at most {String(node.maxItems)} items.
          </span>
        ) : null}
        {atMinimum && items.length === node.minItems ? (
          <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
            This list needs at least {String(node.minItems)} items, so one stays.
          </span>
        ) : null}
      </div>
      {hint !== null ? <FieldHint id={ids.hintId}>{hint}</FieldHint> : null}
      {error !== undefined ? <FieldError id={ids.errorId}>{error}</FieldError> : null}
    </fieldset>
  );
}

/**
 * One array row. A scalar row has no visible label (the fieldset's legend is
 * the label), so the control carries an `aria-label` naming the row instead of
 * relying on a second label element.
 */
function ArrayItem({
  item,
  index,
  node,
  path,
  prefix,
  label,
  disabled,
  onBlur,
  onChange,
}: {
  item: SettingsValue;
  index: number;
  node: SettingsSchemaNode;
  path: string;
  prefix: string;
  label: string;
  disabled: boolean;
  onBlur: (path: string) => void;
  onChange: (next: SettingsValue) => void;
}): ReactNode {
  const ids = fieldIds(path, prefix, false, false);
  const rowLabel = `${label} item ${String(index + 1)}`;
  if (Array.isArray(node.enum)) {
    const enumValues = node.enum;
    const selected = enumValues.findIndex((candidate) => candidate === item);
    return (
      <Select
        value={selected >= 0 ? String(selected) : undefined}
        disabled={disabled}
        onValueChange={(next) => {
          const candidate = enumValues[Number(next)];
          if (candidate !== undefined) onChange(candidate);
          onBlur(path);
        }}
      >
        <SelectTrigger id={ids.controlId} aria-label={rowLabel}>
          <SelectValue placeholder="Choose one" />
        </SelectTrigger>
        <SelectContent>
          {enumValues.map((candidate, position) => (
            <SelectItem key={String(position)} value={String(position)}>
              {String(candidate)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    );
  }
  if (node.type === "boolean") {
    return (
      <Switch
        id={ids.controlId}
        aria-label={rowLabel}
        checked={item === true}
        disabled={disabled}
        onCheckedChange={(checked) => {
          onChange(checked);
          onBlur(path);
        }}
      />
    );
  }
  if (node.type === "integer" || node.type === "number") {
    return (
      <Input
        id={ids.controlId}
        aria-label={rowLabel}
        type="number"
        step={node.type === "integer" ? 1 : "any"}
        min={node.minimum}
        max={node.maximum}
        value={textOf(item)}
        disabled={disabled}
        onBlur={() => {
          onBlur(path);
        }}
        onChange={(event) => {
          onChange(numberFromText(event.target.value) ?? "");
        }}
      />
    );
  }
  return (
    <Input
      id={ids.controlId}
      aria-label={rowLabel}
      type="text"
      value={textOf(item)}
      disabled={disabled}
      onBlur={() => {
        onBlur(path);
      }}
      onChange={(event) => {
        onChange(event.target.value);
      }}
    />
  );
}

function blankFor(node: SettingsSchemaNode): SettingsValue {
  if (Array.isArray(node.enum) && node.enum.length > 0) return node.enum[0] ?? "";
  switch (node.type) {
    case "object": {
      const blank: SettingsObject = {};
      for (const [name, child] of Object.entries(node.properties ?? {})) {
        if (node.required?.includes(name) === true) blank[name] = blankFor(child);
      }
      return blank;
    }
    case "array":
      return [];
    case "number":
    case "integer":
      return node.minimum ?? 0;
    case "boolean":
      return false;
    default:
      return "";
  }
}
