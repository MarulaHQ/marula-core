// SchemaForm — the settings renderer (§9; connector-specification §3.1).
//
// These tests pin the contract that matters: exactly the subset the server
// accepts is rendered, a `writeOnly` field never shows a stored value and is
// reported separately, and client validation refuses what the server refuses.
// A renderer that is one keyword ahead or behind the validator publishes a form
// the API cannot save.
import { describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  SchemaForm,
  readSettingsSchema,
  validateSettingsValue,
  writeOnlyKeys,
  type SecretDrafts,
  type SettingsObject,
  type SettingsSchemaNode,
} from "../../src/components/schema-form";

const SCHEMA: SettingsSchemaNode = {
  type: "object",
  required: ["controller_url", "enabled"],
  properties: {
    controller_url: { type: "string", minLength: 8, maxLength: 200, pattern: "^https://" },
    port: { type: "integer", minimum: 1, maximum: 65535 },
    ratio: { type: "number", minimum: 0, maximum: 1 },
    enabled: { type: "boolean" },
    mode: { type: "string", enum: ["poll", "push"] },
    tags: { type: "array", items: { type: "string" }, minItems: 1, maxItems: 2 },
    network: {
      type: "object",
      required: ["host"],
      properties: {
        host: { type: "string", title: "API host" },
        retries: { type: "integer", minimum: 0, maximum: 5 },
      },
    },
    api_key: { type: "string", writeOnly: true, minLength: 8 },
  },
};

function Harness({
  schema = SCHEMA,
  initial = {},
  secretsSet = [],
  onValue,
  onSecrets,
}: {
  schema?: SettingsSchemaNode;
  initial?: SettingsObject;
  secretsSet?: readonly string[];
  onValue?: (value: SettingsObject) => void;
  onSecrets?: (secrets: SecretDrafts) => void;
}): React.JSX.Element {
  const [value, setValue] = useState<SettingsObject>(initial);
  const [secrets, setSecrets] = useState<SecretDrafts>({});
  return (
    <SchemaForm
      schema={schema}
      value={value}
      onChange={(next) => {
        setValue(next);
        onValue?.(next);
      }}
      secrets={secrets}
      onSecretsChange={(next) => {
        setSecrets(next);
        onSecrets?.(next);
      }}
      secretsSet={secretsSet}
      idPrefix="test"
    />
  );
}

describe("SchemaForm rendering", () => {
  it("renders one control per declared property, with labels from the manifest", () => {
    render(<Harness />);
    expect(screen.getByLabelText(/Controller url/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Port/)).toHaveAttribute("type", "number");
    expect(screen.getByLabelText(/Ratio/)).toHaveAttribute("step", "any");
    expect(screen.getByRole("switch", { name: /Enabled/ })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: /Mode/ })).toBeInTheDocument();
    expect(screen.getByText("API host")).toBeInTheDocument();
    expect(screen.getByLabelText(/Retries/)).toBeInTheDocument();
    // The array renders as a fieldset with a legend, not as a free-text box.
    expect(screen.getByRole("group", { name: /Tags/ })).toBeInTheDocument();
  });

  it("writes into the settings object without touching credentials", async () => {
    const user = userEvent.setup();
    const onValue = vi.fn();
    const onSecrets = vi.fn();
    render(<Harness onValue={onValue} onSecrets={onSecrets} />);

    await user.type(screen.getByLabelText(/Controller url/), "https://acme.example");
    expect(onValue).toHaveBeenLastCalledWith({ controller_url: "https://acme.example" });

    await user.click(screen.getByRole("switch", { name: /Enabled/ }));
    expect(onValue).toHaveBeenLastCalledWith(expect.objectContaining({ enabled: true }));

    await user.type(screen.getByLabelText(/credential/i), "s3cret-token");
    expect(onSecrets).toHaveBeenLastCalledWith({ api_key: "s3cret-token" });
    // The credential never entered the settings document.
    expect(onValue).not.toHaveBeenCalledWith(expect.objectContaining({ api_key: "s3cret-token" }));
  });

  it("never pre-fills a credential and says when one is configured", () => {
    render(
      <Harness
        initial={{ controller_url: "https://acme.example", api_key: "should-never-render" }}
        secretsSet={["api_key"]}
      />,
    );
    const secret = screen.getByLabelText(/credential/i);
    expect(secret).toHaveAttribute("type", "password");
    expect(secret).toHaveValue("");
    expect(secret).toHaveAttribute("placeholder", "Configured — type to replace");
    expect(screen.getByText(/A value is stored/)).toBeInTheDocument();
  });

  it("adds and removes array items within the declared bounds", async () => {
    const user = userEvent.setup();
    render(<Harness initial={{ tags: ["alpha"] }} />);
    const add = screen.getByRole("button", { name: "Add item" });
    await user.click(add);
    expect(screen.getByLabelText("Tags item 2")).toBeInTheDocument();
    // maxItems is 2, so the add control is disabled with the reason beside it.
    expect(screen.getByRole("button", { name: "Add item" })).toBeDisabled();
    expect(screen.getByText(/at most 2 items/)).toBeInTheDocument();
  });

  it("validates on blur and associates the error with the field", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const url = screen.getByLabelText(/Controller url/);
    await user.type(url, "ftp://nope");
    await user.tab();

    const error = await screen.findByText(/does not match its declared pattern/);
    expect(url).toHaveAttribute("aria-invalid", "true");
    expect(url.getAttribute("aria-describedby")).toContain(error.id);
  });

  it("reports an unrenderable schema instead of drawing half a form", () => {
    const refused = readSettingsSchema({
      type: "object",
      properties: { x: { type: "string", oneOf: [] } },
    });
    expect(refused.ok).toBe(false);
    if (!refused.ok) expect(refused.reason).toMatch(/oneOf/);

    const tooDeep = readSettingsSchema({
      type: "object",
      properties: {
        a: {
          type: "object",
          properties: {
            b: {
              type: "object",
              properties: {
                c: {
                  type: "object",
                  properties: {
                    d: { type: "object", properties: { e: { type: "string" } } },
                  },
                },
              },
            },
          },
        },
      },
    });
    expect(tooDeep.ok).toBe(false);

    const noItems = readSettingsSchema({
      type: "object",
      properties: { names: { type: "array" } },
    });
    expect(noItems.ok).toBe(false);
  });

  it("accepts the annotations the server accepts and ignores", () => {
    const read = readSettingsSchema({
      type: "object",
      properties: {
        bucket: {
          type: "string",
          title: "Bucket",
          description: "Where exports land",
          default: "invoices",
          format: "uri",
          minLength: 1,
        },
      },
    });
    expect(read.ok).toBe(true);
  });
});

describe("validateSettingsValue", () => {
  it("refuses an unknown key at any object level", () => {
    const errors = validateSettingsValue(SCHEMA, {
      controller_url: "https://acme.example",
      enabled: true,
      surprise: "x",
    });
    expect(errors.surprise).toMatch(/not declared/);

    const nested = validateSettingsValue(SCHEMA, {
      controller_url: "https://acme.example",
      enabled: true,
      network: { host: "a", mystery: 1 },
    });
    expect(nested["network.mystery"]).toMatch(/not declared/);
  });

  it("refuses a credential sent through settings, and an unknown secret", () => {
    const inSettings = validateSettingsValue(SCHEMA, {
      controller_url: "https://acme.example",
      enabled: true,
      api_key: "leak",
    });
    expect(inSettings.api_key).toMatch(/credential/);

    const wrongSecret = validateSettingsValue(
      SCHEMA,
      { controller_url: "https://acme.example", enabled: true },
      { not_a_credential: "x" },
    );
    expect(wrongSecret.not_a_credential).toMatch(/not a credential field/);
  });

  it("counts a configured credential as satisfying required, and a draft as a value", () => {
    const schema: SettingsSchemaNode = {
      type: "object",
      required: ["token"],
      properties: { token: { type: "string", writeOnly: true } },
    };
    expect(Object.keys(validateSettingsValue(schema, {}, {}, []))).toEqual(["token"]);
    expect(Object.keys(validateSettingsValue(schema, {}, {}, ["token"]))).toEqual([]);
    expect(Object.keys(validateSettingsValue(schema, {}, { token: "t" }, []))).toEqual([]);
  });

  it("mirrors the numeric, length and array bounds", () => {
    const base = { controller_url: "https://acme.example", enabled: true };
    expect(
      validateSettingsValue(SCHEMA, { ...base, port: 70000 }).port,
    ).toMatch(/maximum/);
    expect(validateSettingsValue(SCHEMA, { ...base, port: 1.5 }).port).toMatch(/whole number/);
    expect(
      validateSettingsValue(SCHEMA, { ...base, controller_url: "https:/x" }).controller_url,
    ).toMatch(/shorter than|does not match/);
    expect(
      validateSettingsValue(SCHEMA, { ...base, tags: [] }).tags,
    ).toMatch(/at least 1 item/);
    expect(
      validateSettingsValue(SCHEMA, { ...base, tags: ["a", "b", "c"] }).tags,
    ).toMatch(/at most 2 items/);
    expect(validateSettingsValue(SCHEMA, { ...base, enabled: "yes" }).enabled).toMatch(
      /true or false/,
    );
    expect(validateSettingsValue(SCHEMA, { ...base, mode: "carrier-pigeon" }).mode).toMatch(
      /declared values/,
    );
  });

  it("lists the top-level writeOnly fields and only those", () => {
    expect(writeOnlyKeys(SCHEMA)).toEqual(["api_key"]);
    expect(
      writeOnlyKeys({
        type: "object",
        properties: { nested: { type: "object", properties: { token: { type: "string", writeOnly: true } } } },
      }),
    ).toEqual([]);
  });
});
