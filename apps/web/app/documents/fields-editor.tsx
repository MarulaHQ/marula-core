// Custom-field editor + input parsing shared by the documents area and
// the asset pages (lane 5 extracted it out of the documents page module —
// Next.js forbids non-page exports from app/*/page.tsx).
"use client";

import { Input, Label, Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@marula-core/ui";
import type { components } from "@marula-core/sdk";

type DocumentLayout = components["schemas"]["DocumentLayout"];
type LayoutField = components["schemas"]["LayoutField"];

export function FieldsEditor({
  layout,
  fields,
  onChange,
}: {
  layout: DocumentLayout;
  fields: Record<string, string>;
  onChange: (next: Record<string, string>) => void;
}) {
  const set = (key: string, value: string) => onChange({ ...fields, [key]: value });
  const sorted = [...layout.fields].sort((a, b) => (a.position ?? 0) - (b.position ?? 0));
  return (
    <div className="grid gap-2">
      {sorted.map((f) => (
        <div key={f.key}>
          <Label htmlFor={`f-${f.key}`}>
            {f.label}
            {f.required ? " *" : ""}
          </Label>
          {f.type === "select" ? (
            <Select value={fields[f.key] ?? ""} onValueChange={(v) => set(f.key, v)}>
              <SelectTrigger id={`f-${f.key}`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="">—</SelectItem>
                {(f.options ?? []).map((o) => (
                  <SelectItem key={o} value={o}>
                    {o}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          ) : f.type === "bool" ? (
            <Select value={fields[f.key] ?? ""} onValueChange={(v) => set(f.key, v)}>
              <SelectTrigger id={`f-${f.key}`}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="">—</SelectItem>
                <SelectItem value="true">Yes</SelectItem>
                <SelectItem value="false">No</SelectItem>
              </SelectContent>
            </Select>
          ) : (
            <Input
              id={`f-${f.key}`}
              type={f.type === "date" ? "date" : f.type === "number" ? "number" : "text"}
              value={fields[f.key] ?? ""}
              onChange={(e) => set(f.key, e.target.value)}
            />
          )}
        </div>
      ))}
    </div>
  );
}

export function parseFieldInputs(
  schema: LayoutField[],
  fields: Record<string, string>,
): { values: Record<string, unknown> } | { error: string } {
  const values: Record<string, unknown> = {};
  for (const f of schema) {
    const raw = fields[f.key]?.trim() ?? "";
    if (raw === "") {
      if (f.required) return { error: `Field “${f.label}” is required.` };
      continue;
    }
    if (f.type === "number") {
      const n = Number(raw);
      if (!Number.isFinite(n)) return { error: `Field “${f.label}” must be a number.` };
      values[f.key] = n;
    } else if (f.type === "bool") {
      if (raw !== "true" && raw !== "false")
        return { error: `Field “${f.label}” must be yes or no.` };
      values[f.key] = raw === "true";
    } else {
      values[f.key] = raw;
    }
  }
  return { values };
}

