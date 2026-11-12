// Shared fork (duplicate) dialog for the template editor surfaces.
//
// Prefills a unique-ish copy name and slug, mirrors the server's slug rule
// client-side so an invalid slug never leaves the browser, and surfaces a
// typed rejection (slug taken, key reused) as friendly inline copy.
"use client";

import { useState } from "react";
import {
  Button,
  Card,
  CardContent,
  CardHeader,
  FieldError,
  FieldHint,
  Input,
} from "@marula-core/ui";
import { forkTemplate } from "../../../lib/email-templates/actions";
import type { TemplateRow } from "../../../lib/email-templates/types";

/** Slug rule the server enforces (routes/templates.rs `validate_slug`). */
export function slugValid(slug: string): boolean {
  return /^[a-z0-9][a-z0-9_-]{0,63}$/.test(slug);
}

export function ForkDialog({
  row,
  onClose,
  onForked,
  /** Present when the dialog opened from an editor: jump to the copy. */
  onOpenCopy,
}: {
  row: TemplateRow;
  onClose: () => void;
  onForked: () => void;
  onOpenCopy?: (id: string) => void;
}): React.JSX.Element {
  const [name, setName] = useState(`${row.name} (copy)`);
  const [slug, setSlug] = useState(`${row.slug}-copy`);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const slugOk = slugValid(slug.trim());
  const nameOk = name.trim() !== "" && name.length <= 128;

  const submit = async () => {
    setBusy(true);
    setError(null);
    const result = await forkTemplate(row.id, { name: name.trim(), slug: slug.trim() });
    setBusy(false);
    if (!result.ok) {
      setError(result.friendly ?? "The copy was refused.");
      return;
    }
    onForked();
    if (result.template && onOpenCopy) onOpenCopy(result.template.id);
    else onClose();
  };

  return (
    <div
      role="dialog"
      aria-label={`Duplicate ${row.name}`}
      className="fixed inset-0 z-50 grid place-items-center p-4"
      style={{ background: "color-mix(in oklab, var(--m-text) 40%, transparent)" }}
    >
      <Card className="w-full max-w-lg">
        <CardHeader>
          <div>
            <h3 className="text-sm font-semibold">Duplicate {row.name}</h3>
            <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              Makes an editable copy in your workspace. The original stays as it is.
            </p>
          </div>
        </CardHeader>
        <CardContent>
          <div className="grid gap-3">
            <label className="grid gap-1">
              <span className="text-xs">Name</span>
              <Input value={name} onChange={(e) => setName(e.target.value)} maxLength={128} />
            </label>
            <label className="grid gap-1">
              <span className="text-xs">Slug</span>
              <Input value={slug} onChange={(e) => setSlug(e.target.value)} maxLength={64} />
              <FieldHint>
                Lowercase letters, numbers, dashes or underscores. Unique in the workspace.
              </FieldHint>
            </label>
            {error ? <FieldError>{error}</FieldError> : null}
            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={onClose} disabled={busy}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={() => void submit()}
                disabled={busy || !slugOk || !nameOk}
              >
                {busy ? "Duplicating…" : onOpenCopy ? "Duplicate and edit" : "Duplicate"}
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
