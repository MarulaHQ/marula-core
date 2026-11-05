"use client";
/**
 * DescribeDraftPanel — "describe it" → a reviewed report draft, as one panel.
 *
 * The three surfaces that offer this (the /reports entry, the /dashboards
 * entry, and the /reports/author route) share this state machine, so a
 * transcript cannot behave one way on one page and another way on the next.
 *
 * The one leg:
 *   1. text in, `POST /api/v1/reports/author` out;
 *   2. the server's validated draft, its typed issues, its plain-English
 *      read-back and the diff against the current draft;
 *   3. accept / edit / reject. Accept saves through the EXISTING definition
 *      routes (`acceptReportDraft`), so the typed 422 and the registry stay the
 *      only authority on what may be stored. Edit hands the draft's spec to the
 *      caller, which opens the one builder.
 *
 * A refusal that never reached a model (AI off, no grant, a cap, no credential,
 * no allowlisted model) renders as `AiGateNotice` with its remedy, never as a
 * draft and never as a disabled button with no explanation.
 *
 * Non-purpose: no persistence of a draft. Wave 46 gives the draft no table, so
 * it lives in this state and is gone when the panel is.
 */
import { Callout, Icon } from "@marula-core/ui/kit";
import { useCallback, useState } from "react";
import {
  acceptReportDraft,
  authorReportDraft,
  draftStatusOf,
  type AuthorReportResponse,
  type CurrentDraft,
  type DraftStatus,
  type ReportDraft,
} from "../../lib/report-authoring";
import { TranscriptPanel } from "./transcript-panel";
import { DraftRow } from "./draft-row";
import { AiGateNotice } from "./ai-gate-notice";

export interface DescribeDraftPanelProps {
  /** The report being edited, when the entry was opened on one. */
  current?: CurrentDraft | null;
  /** Which surface this panel is on. Only used for the refusal state's label. */
  surface: "reports" | "dashboards";
  /** Edit: the caller opens the one builder with this draft's spec. */
  onEdit: (draft: ReportDraft) => void;
  /** Accept succeeded: the definition now exists with this id. */
  onAccepted?: (result: { id: string; created: boolean; draft: ReportDraft }) => void;
  /** Where "open the reports list" should go. */
  onOpenReports?: () => void;
}

export function DescribeDraftPanel({
  current = null,
  surface,
  onEdit,
  onAccepted,
  onOpenReports,
}: DescribeDraftPanelProps) {
  const [transcript, setTranscript] = useState("");
  const [status, setStatus] = useState<DraftStatus>("idle");
  const [answer, setAnswer] = useState<AuthorReportResponse | null>(null);
  const [refusal, setRefusal] = useState<{ code: string; message: string } | null>(null);
  const [name, setName] = useState(current?.name ?? "");
  const [accepting, setAccepting] = useState(false);
  const [acceptError, setAcceptError] = useState<string | null>(null);
  const [accepted, setAccepted] = useState<{ id: string; created: boolean } | null>(null);

  const generate = useCallback(async () => {
    setStatus("drafting");
    setRefusal(null);
    setAcceptError(null);
    setAccepted(null);
    setAnswer(null);
    const outcome = await authorReportDraft({ transcript, current });
    if (!outcome.ok) {
      setRefusal({ code: outcome.refusal, message: outcome.message });
      setStatus("refused");
      return;
    }
    setAnswer(outcome.result);
    if (outcome.result.draft?.name !== undefined && name.trim() === "") {
      setName(outcome.result.draft.name);
    }
    setStatus(draftStatusOf(outcome.result, false));
  }, [transcript, current, name]);

  const reject = useCallback(() => {
    setAnswer(null);
    setRefusal(null);
    setAcceptError(null);
    setAccepted(null);
    setStatus("rejected");
  }, []);

  const accept = useCallback(async () => {
    if (answer?.draft === undefined) return;
    const draft = answer.draft;
    setAccepting(true);
    setAcceptError(null);
    const outcome = await acceptReportDraft({
      draft,
      name,
      currentId: current?.id ?? null,
    });
    setAccepting(false);
    if (!outcome.ok) {
      setAcceptError(outcome.message);
      return;
    }
    setAccepted({ id: outcome.id, created: outcome.created });
    setStatus("accepted");
    onAccepted?.({ id: outcome.id, created: outcome.created, draft });
  }, [answer, name, current?.id, onAccepted]);

  const showRow = status !== "idle" && status !== "rejected";

  return (
    <div className="grid gap-4">
      <TranscriptPanel
        value={transcript}
        onChange={setTranscript}
        onGenerate={() => void generate()}
        busy={status === "drafting"}
        disabled={transcript.trim() === ""}
      />

      {status === "refused" && refusal !== null ? (
        <AiGateNotice
          reason={refusal.code}
          message={refusal.message}
          surface={surface}
        />
      ) : null}

      {showRow ? (
        <DraftRow
          status={status}
          draft={answer?.draft ?? null}
          issues={answer?.issues ?? []}
          diff={answer?.diff ?? []}
          verification={answer?.verification ?? null}
          // The gate notice above carries the refusal in its own words; the row
          // keeps the status word so the two do not print the same sentence.
          refusalMessage={null}
          name={name}
          onNameChange={setName}
          onAccept={() => void accept()}
          onEdit={() => {
            if (answer?.draft !== undefined) onEdit(answer.draft);
          }}
          onReject={reject}
          accepting={accepting}
          acceptError={acceptError}
        />
      ) : null}

      {status === "rejected" ? (
        <Callout icon={<Icon name="info" size={14} />}>
          The draft is gone. Nothing was stored, so there is nothing to undo.
          Describe it again, or open the builder.
        </Callout>
      ) : null}

      {accepted !== null ? (
        <Callout icon={<Icon name="check" size={14} />}>
          {accepted.created
            ? "Saved as a new private report."
            : "The saved report was updated."}{" "}
          {onOpenReports !== undefined ? (
            <button
              type="button"
              className="font-semibold underline underline-offset-2"
              onClick={onOpenReports}
            >
              Open the reports list
            </button>
          ) : null}
        </Callout>
      ) : null}
    </div>
  );
}
