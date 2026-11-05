"use client";
/**
 * DescribeEntry — the "describe it" door on a surface that already exists.
 *
 * A button that opens a dialog holding `DescribeDraftPanel`. The dialog is the
 * point: the reports and dashboards pages keep their own density and tab
 * structure, and the authoring panel is borrowed whole rather than copied into
 * either page (which is how the two would drift).
 *
 * The entry never writes by itself. Accept inside the panel goes through the
 * existing definition routes; Edit closes the dialog and hands the draft to the
 * caller, which opens the one builder. `/reports/author` still exists for the
 * narrower authoring route and is not replaced by this.
 *
 * Non-purpose: no audio, no second save path, no draft storage.
 */
import { Button, Dialog, DialogContent, DialogTrigger } from "@marula-core/ui";
import { useState } from "react";
import type { CurrentDraft, ReportDraft } from "../../lib/report-authoring";
import { DescribeDraftPanel } from "./describe-draft-panel";

export interface DescribeEntryProps {
  surface: "reports" | "dashboards";
  /** The definition being edited, when the entry is opened on a saved report. */
  current?: CurrentDraft | null;
  /** Button label. Defaults to the surface's own wording. */
  label?: string;
  onEdit: (draft: ReportDraft) => void;
  onAccepted?: (result: { id: string; created: boolean; draft: ReportDraft }) => void;
  onOpenReports?: () => void;
  /** Test hook for the trigger. */
  testId?: string;
}

export function DescribeEntry({
  surface,
  current = null,
  label,
  onEdit,
  onAccepted,
  onOpenReports,
  testId,
}: DescribeEntryProps) {
  const [open, setOpen] = useState(false);
  // Remounting the panel on each open keeps a rejected or accepted draft from
  // greeting the next visit as if it were still live.
  const [run, setRun] = useState(0);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setOpen(next);
        if (next) setRun((n) => n + 1);
      }}
    >
      <DialogTrigger asChild>
        <Button variant="secondary" size="sm" data-testid={testId ?? "describe-entry"}>
          {label ?? (surface === "dashboards" ? "Describe a card" : "Describe a report")}
        </Button>
      </DialogTrigger>
      <DialogContent
        title={surface === "dashboards" ? "Describe a card" : "Describe a report"}
        description="Say what it should answer. The draft is checked against the same registry the builder saves through, and nothing is stored until you accept it."
        width={880}
      >
        <div className="p-4">
          <DescribeDraftPanel
            key={run}
            current={current}
            surface={surface}
            onEdit={(draft) => {
              setOpen(false);
              onEdit(draft);
            }}
            onAccepted={onAccepted}
            onOpenReports={
              onOpenReports === undefined
                ? undefined
                : () => {
                    setOpen(false);
                    onOpenReports();
                  }
            }
          />
        </div>
      </DialogContent>
    </Dialog>
  );
}
