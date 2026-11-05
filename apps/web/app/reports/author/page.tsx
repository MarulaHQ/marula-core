// /reports/author — "describe it" → a reviewed report draft (Wave 46 lane 3).
//
// Why this route and not the /reports page: `/reports` is row 6's file, and its
// own header says "no AI affordance anywhere on this surface". This lane owns
// the authoring surface, so it lives at its own route and row 6 links to it from
// the reports header. The seam between them is the EXISTING builder deep link
// (`/reports?spec=<encoded>`): "Edit in the builder" hands the draft to the
// builder that already exists rather than growing a second one, and the `?spec=`
// it lands on is the format `lib/report-spec-codec` already defines.
//
// What this page is:
//   * a transcript field (text, never audio — voice capture is another lane);
//   * the draft as ONE row with a status, the server's plain-English read-back,
//     the typed issues, and the structural diff against the current draft;
//   * accept / edit / reject. Accept saves through the existing definition
//     routes, edit opens the existing builder, reject discards client state.
//
// What it is not: a second builder, a second validator, a stored draft. Wave 46
// gives this row no migration, so a draft exists only as the answer to one
// request plus the state on this page, and it is gone when the page is.

"use client";

export const dynamic = "force-dynamic";

import { Button, SurfaceHeader } from "@marula-core/ui";
import { Callout, Icon, Pill } from "@marula-core/ui/kit";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { useRequireAuth } from "../../../lib/api";
import { decodeSpecFromQuery, encodeSpecToQuery } from "../../../lib/report-spec-codec";
import {
  acceptReportDraft,
  authorReportDraft,
  draftStatusOf,
  type AuthorReportResponse,
  type CurrentDraft,
  type DraftStatus,
} from "../../../lib/report-authoring";
import { DraftRow } from "../../../components/reports/draft-row";
import { TranscriptPanel } from "../../../components/reports/transcript-panel";

/** What a definition deep link carries: the spec, its id, and its name. */
interface LandedDraft {
  id?: string;
  name?: string;
  spec: CurrentDraft["spec"];
}

export default function ReportAuthorPage() {
  return (
    <Suspense fallback={null}>
      <ReportAuthorInner />
    </Suspense>
  );
}

function ReportAuthorInner() {
  const auth = useRequireAuth();
  const router = useRouter();
  const searchParams = useSearchParams();

  const [transcript, setTranscript] = useState("");
  const [status, setStatus] = useState<DraftStatus>("idle");
  const [answer, setAnswer] = useState<AuthorReportResponse | null>(null);
  const [refusal, setRefusal] = useState<{ code: string; message: string } | null>(null);
  const [name, setName] = useState("");
  const [accepting, setAccepting] = useState(false);
  const [acceptError, setAcceptError] = useState<string | null>(null);
  const [accepted, setAccepted] = useState<{ id: string; created: boolean } | null>(null);

  // The current draft, when the caller deep-linked one. `?spec=` is the
  // builder's own encoding; `?definition=` and `?name=` ride along so an accept
  // updates that definition instead of creating a duplicate.
  const landed = useMemo<LandedDraft | null>(() => {
    const decoded = decodeSpecFromQuery(searchParams.get("spec"));
    if (decoded === null) return null;
    return {
      id: searchParams.get("definition") ?? undefined,
      name: searchParams.get("name") ?? undefined,
      spec: decoded.spec,
    };
  }, [searchParams]);

  useEffect(() => {
    if (landed?.name !== undefined) setName(landed.name);
  }, [landed?.name]);

  /**
   * The base the request sends. A stored report definition has a spec and no
   * card, so no card is sent: the server reports the card members as additions
   * rather than diffing against a card nobody chose.
   */
  const current: CurrentDraft | null = useMemo(
    () =>
      landed === null
        ? null
        : { id: landed.id, name: landed.name, spec: landed.spec },
    [landed],
  );

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
    setAccepting(true);
    setAcceptError(null);
    const outcome = await acceptReportDraft({
      draft: answer.draft,
      name,
      currentId: landed?.id ?? null,
    });
    setAccepting(false);
    if (!outcome.ok) {
      setAcceptError(outcome.message);
      return;
    }
    setAccepted({ id: outcome.id, created: outcome.created });
    setStatus("accepted");
  }, [answer, name, landed?.id]);

  /** Edit hands the draft to the builder that already exists, through its deep link. */
  const edit = useCallback(() => {
    if (answer?.draft === undefined) return;
    router.push(`/reports?spec=${encodeSpecToQuery(answer.draft.spec)}`);
  }, [answer, router]);

  if (auth.status !== "authed") return null;

  const draft = answer?.draft ?? null;
  const showRow = status !== "idle" && status !== "rejected";

  return (
    <div className="grid gap-5 p-6">
      <SurfaceHeader
        className="animate-[m-rise_120ms_cubic-bezier(0.2,0,0,1)] motion-reduce:animate-none"
        title="Describe a report"
        eyebrow="Reports"
        meta={
          <>
            <Pill tone="neutral" size="lg">
              Draft only
            </Pill>
            {landed !== null ? (
              <Pill tone="gold" size="lg">
                {landed.id === undefined ? "New report" : "Editing a saved report"}
              </Pill>
            ) : null}
          </>
        }
        actions={
          <Button variant="secondary" onClick={() => router.push("/reports")}>
            Back to reports
          </Button>
        }
      />

      <div className="grid items-start gap-5 xl:grid-cols-[minmax(0,1fr)_320px]">
        <div className="grid gap-5">
          <TranscriptPanel
            value={transcript}
            onChange={setTranscript}
            onGenerate={() => void generate()}
            busy={status === "drafting"}
            disabled={transcript.trim() === ""}
          />

          {showRow ? (
            <DraftRow
              status={status}
              draft={draft}
              issues={answer?.issues ?? []}
              diff={answer?.diff ?? []}
              refusalMessage={refusal?.message ?? null}
              name={name}
              onNameChange={setName}
              onAccept={() => void accept()}
              onEdit={edit}
              onReject={reject}
              accepting={accepting}
              acceptError={acceptError}
            />
          ) : null}

          {status === "rejected" ? (
            <Callout icon={<Icon name="info" size={14} />}>
              The draft is gone. Nothing was stored, so there is nothing to undo —
              describe it again, or open the builder.
            </Callout>
          ) : null}

          {accepted !== null ? (
            <Callout icon={<Icon name="check" size={14} />}>
              {accepted.created
                ? "Saved as a new private report."
                : "The saved report was updated."}{" "}
              <button
                type="button"
                className="font-semibold underline underline-offset-2"
                onClick={() => router.push("/reports")}
              >
                Open it in the reports list
              </button>
              .
            </Callout>
          ) : null}
        </div>

        <aside className="grid gap-3" aria-label="Draft context">
          <section
            className="grid gap-2 rounded-[16px] border p-4"
            style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
          >
            <h2 className="text-[13px] font-semibold">The current draft</h2>
            {landed === null ? (
              <p className="text-[13px] leading-[1.5] text-muted-foreground">
                None. This proposal is new, so the diff against it is every part of
                the report.
              </p>
            ) : (
              <dl className="grid gap-1.5 text-[13px]">
                <Fact label="Report" value={landed.name ?? "Untitled"} />
                <Fact label="View" value={landed.spec.view} />
                <Fact label="Viz" value={landed.spec.viz} />
                <Fact label="Measures" value={String(landed.spec.measures.length)} />
                <Fact label="Dimensions" value={String(landed.spec.group_by?.length ?? 0)} />
              </dl>
            )}
          </section>

          {answer !== null ? (
            <section
              className="grid gap-2 rounded-[16px] border p-4"
              style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
            >
              <h2 className="text-[13px] font-semibold">What the model was told</h2>
              <dl className="grid gap-1.5 text-[13px]">
                <Fact label="Views catalogued" value={String(answer.catalogue.views)} />
                <Fact label="Shapes described" value={String(answer.catalogue.shapes)} />
                <Fact label="Card rules" value={String(answer.catalogue.rules)} />
                <Fact label="Prompt" value={`${answer.catalogue.prompt_chars} chars`} />
                <Fact label="Steps" value={String(answer.usage.steps)} />
              </dl>
              <p className="text-[12px] leading-[1.5] text-muted-foreground">
                Columns are not in the prompt — the model fetches the ones it needs
                {answer.tool_calls.length > 0
                  ? `; it asked for ${answer.tool_calls
                      .map((call) => call.argument)
                      .filter((arg) => arg !== "")
                      .join(", ")}`
                  : ""}
                .
              </p>
              <p className="text-[12px] leading-[1.5] text-muted-foreground">
                {answer.model_info.provider} · {answer.model_info.model} ·{" "}
                {answer.model_info.resolution}
              </p>
            </section>
          ) : null}

          <Callout icon={<Icon name="info" size={14} />}>
            A saved report stores its query. The card&apos;s window, refresh and size
            travel with this draft and belong to the surface that displays it — the
            report definition has no card field today.
          </Callout>
        </aside>
      </div>
    </div>
  );
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="min-w-0 truncate font-mono text-[12px] tabular-nums">{value}</dd>
    </div>
  );
}
