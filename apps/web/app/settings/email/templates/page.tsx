// /settings/email/templates — the admin template editor's index.
//
// Purpose: every template row in one place, grouped by kind (email first),
// with the honest provenance the API exposes — a shipped default
// (`is_system` + `builtin`), a fork (`forked_from`), or a custom org row —
// and the three row actions the API supports: edit, duplicate (fork),
// reset-to-default (restore). The list is unfiltered, so templates a newer
// backend seeds (new slugs, new kinds) appear here without a client change.
//
// Non-purpose: no rendering here (the editor route owns the draft and the
// preview) and no create-from-scratch — duplication via fork is the create
// path the API offers.
"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  FieldError,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
  formatInTimeZone,
} from "@marula-core/ui";
import { api, useRequireAuth } from "../../../../lib/api";
import { useTimeFormat } from "../../../../lib/use-time-format";
import { ConfirmDestructive, SettingsHeader } from "../../../../components/settings/settings-shell";
import { ForkDialog } from "../../../../components/settings/email-templates/fork-dialog";
import { restoreTemplate } from "../../../../lib/email-templates/actions";
import {
  TEMPLATE_KIND_ORDER,
  templateKindLabel,
} from "../../../../lib/email-templates/variables";
import type { TemplateRow, TemplatesList } from "../../../../lib/email-templates/types";

/** Rows grouped by kind, email first, unknown kinds after the known order. */
function groupByKind(rows: TemplateRow[]): [string, TemplateRow[]][] {
  const groups = new Map<string, TemplateRow[]>();
  for (const row of rows) {
    const bucket = groups.get(row.kind);
    if (bucket) bucket.push(row);
    else groups.set(row.kind, [row]);
  }
  const known = TEMPLATE_KIND_ORDER.filter((kind) => groups.has(kind));
  const extra = [...groups.keys()].filter(
    (kind) => !(TEMPLATE_KIND_ORDER as readonly string[]).includes(kind),
  );
  return [...known, ...extra].map((kind) => [kind, groups.get(kind) ?? []]);
}

export default function EmailTemplatesPage() {
  const auth = useRequireAuth();
  const { timeZone } = useTimeFormat();
  const [list, setList] = useState<TemplatesList | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);

  const load = useCallback(async () => {
    const res = await api.api.GET("/api/v1/templates", {
      params: { query: { limit: 200 } },
    });
    if (res.data) setList(res.data as unknown as TemplatesList);
    if (res.error) setLoadError("The templates could not be loaded.");
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    void load();
  }, [auth.status, load, refreshTick]);

  if (auth.status !== "authed") return null;
  if (!list) {
    return (
      <div className="grid gap-4">
        <SettingsHeader title="Email templates" description="Loading the templates…" />
        {loadError ? (
          <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
            {loadError}
          </p>
        ) : (
          <Spinner />
        )}
      </div>
    );
  }

  const groups = groupByKind(list.templates);

  return (
    <div className="grid gap-6">
      <SettingsHeader
        title="Email templates"
        description="What outbound mail says. Built-in templates are workspace defaults — duplicate one to make it yours. Placeholders like {{client.name}} fill in when a message sends."
      >
        <Badge tone="neutral">{list.templates.length}</Badge>
      </SettingsHeader>

      {groups.length === 0 ? (
        <p className="text-[13px]" style={{ color: "var(--m-text-muted)" }}>
          No templates yet. Templates your workspace ships with appear here.
        </p>
      ) : (
        groups.map(([kind, rows]) => (
          <KindCard key={kind} kind={kind} rows={rows} timeZone={timeZone} onChanged={() => setRefreshTick((n) => n + 1)} />
        ))
      )}
    </div>
  );
}

function KindCard({
  kind,
  rows,
  timeZone,
  onChanged,
}: {
  kind: string;
  rows: TemplateRow[];
  timeZone: string;
  onChanged: () => void;
}) {
  return (
    <Card>
      <CardHeader>
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold">{templateKindLabel(kind)}</h2>
          <Badge tone="outline">{rows.length}</Badge>
        </div>
      </CardHeader>
      <CardContent>
        <div className="rounded-md border overflow-hidden" style={{ borderColor: "var(--m-border)" }}>
          <Table>
            <thead>
              <tr
                className="text-left text-[11px] uppercase tracking-wide"
                style={{ color: "var(--m-text-muted)", background: "var(--m-canvas)" }}
              >
                <Th className="font-medium">Template</Th>
                <Th className="font-medium">Slug</Th>
                <Th className="font-medium">Status</Th>
                <Th className="font-medium">Updated</Th>
                <Th className="font-medium">Actions</Th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <TemplateRowItem key={row.id} row={row} timeZone={timeZone} onChanged={onChanged} />
              ))}
            </tbody>
          </Table>
        </div>
      </CardContent>
    </Card>
  );
}

function TemplateRowItem({
  row,
  timeZone,
  onChanged,
}: {
  row: TemplateRow;
  timeZone: string;
  onChanged: () => void;
}) {
  const [forkOpen, setForkOpen] = useState(false);
  const [restoreOpen, setRestoreOpen] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [restoreError, setRestoreError] = useState<string | null>(null);

  // Honest provenance, exactly what the API exposes: is_system + builtin is a
  // shipped default, forked_from is a fork of one, everything else is custom.
  const provenance =
    row.is_system && row.builtin ? (
      <Badge tone="outline">Built-in</Badge>
    ) : row.forked_from ? (
      <Badge tone="warning">Fork of {row.forked_from}</Badge>
    ) : (
      <Badge tone="neutral">Custom</Badge>
    );

  const restorable = row.is_system && row.builtin !== null;

  const confirmRestore = async () => {
    setRestoring(true);
    setRestoreError(null);
    const result = await restoreTemplate(row);
    setRestoring(false);
    if (!result.ok) {
      setRestoreError(result.friendly ?? "The reset was refused.");
      return;
    }
    setRestoreOpen(false);
    onChanged();
  };

  return (
    <Tr className="border-t" style={{ borderColor: "var(--m-border)" }}>
      <Td className="px-3 py-2.5">
        <Link
          href={`/settings/email/templates/${row.id}`}
          className="text-[13px] font-medium hover:underline"
          style={{ color: "var(--m-text)" }}
        >
          {row.name}
        </Link>
        {row.description ? (
          <div className="text-xs" style={{ color: "var(--m-text-secondary)" }}>
            {row.description}
          </div>
        ) : null}
      </Td>
      <Td className="px-3 py-2.5 text-xs" style={{ fontFamily: "var(--m-font-mono)" }}>
        {row.slug}
      </Td>
      <Td className="px-3 py-2.5">{provenance}</Td>
      <Td className="px-3 py-2.5 text-xs" style={{ whiteSpace: "nowrap" }}>
        {formatInTimeZone(row.updated_at, timeZone, "datetime")}
      </Td>
      <Td className="px-3 py-2.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <Button variant="secondary" size="sm" asChild>
            <Link href={`/settings/email/templates/${row.id}`}>Edit</Link>
          </Button>
          <Button variant="secondary" size="sm" onClick={() => setForkOpen(true)}>
            Duplicate
          </Button>
          {restorable ? (
            <Button variant="secondary" size="sm" onClick={() => setRestoreOpen(true)}>
              Reset to default
            </Button>
          ) : null}
        </div>
        {restoreError ? <FieldError>{restoreError}</FieldError> : null}
      </Td>

      {forkOpen ? <ForkDialog row={row} onClose={() => setForkOpen(false)} onForked={onChanged} /> : null}
      <ConfirmDestructive
        open={restoreOpen}
        onOpenChange={setRestoreOpen}
        title={`Reset ${row.name} to default`}
        body="The subject and body go back to the shipped default. Your workspace's edits to this copy are discarded."
        confirmLabel="Reset to default"
        busy={restoring}
        onConfirm={() => void confirmRestore()}
      />
    </Tr>
  );
}
