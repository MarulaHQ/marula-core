"use client";
/**
 * DraftRow — one authored report draft, as a row with a status.
 *
 * The composition comes from the three reference families this lane studied:
 *
 * - **Draft tables** (PandaDoc's documents-with-statuses, Intercom's drafts
 *   table): a draft is a ROW whose first job is to say what state it is in, so
 *   the status sits on the name's baseline as a pill **with a word in it**, and
 *   the row carries one meta line rather than a paragraph.
 * - **Describe-and-review flows** (Craft's Document Review, Pages' "Describe
 *   your change" sheet): the proposal states in plain English what it will do,
 *   the change is shown, and the two decisions are named — accept or reject —
 *   with edit as the third way out. Nothing applies itself.
 * - **Specimen / split-studio pages** (Docker's article with its sticky rail,
 *   Everlane's product panel): labelled facts in a fixed order on the right of
 *   the primary content, so a reviewer's eye can compare two drafts by scanning
 *   the same four positions.
 *
 * Tokens and typography are the product's, not the references': Figtree at the
 * product's sizes, `--m-*` surfaces through the Tailwind bridge, 16px card
 * radius, borders over shadows. Colour is never the only signal — every status
 * carries its word, every op carries its sign.
 */
import { Button, Card, CardContent, CardHeader, Input, Spinner } from "@marula-core/ui";
import { Banner, Icon, Pill } from "@marula-core/ui/kit";
import { DiffList } from "./diff-list";
import {
  DRAFT_STATUS_LABEL,
  DRAFT_STATUS_TONE,
  diffTally,
  issuesOutsideVerification,
  verificationSummary,
  type DraftFinding,
  type DraftStatus,
  type DraftVerification,
  type ReportAuthoringIssue,
  type ReportDiffEntry,
  type ReportDraft,
} from "../../lib/report-authoring";

export interface DraftRowProps {
  status: DraftStatus;
  /** The validated draft. Absent when the answer was refused or rejected. */
  draft: ReportDraft | null;
  issues: ReportAuthoringIssue[];
  diff: ReportDiffEntry[];
  /** The typed refusal sentence (AI off, a cap, no model) when `status` is `refused`. */
  refusalMessage?: string | null;
  /** The name that will be used if the draft is accepted. */
  name: string;
  onNameChange: (next: string) => void;
  onAccept: () => void;
  onEdit: () => void;
  onReject: () => void;
  accepting?: boolean;
  /** A save attempt's failure, in blush — never red, never a stack trace. */
  acceptError?: string | null;
  /**
   * What the server's data check did (Wave 64 lane 5): the classes it checked,
   * what a repair cleared, and what survives. Absent on an answer from a server
   * that predates the check, in which case the row claims nothing about it.
   */
  verification?: DraftVerification | null;
}

export function DraftRow({
  status,
  draft,
  issues,
  diff,
  refusalMessage,
  name,
  onNameChange,
  onAccept,
  onEdit,
  onReject,
  accepting = false,
  acceptError = null,
  verification = null,
}: DraftRowProps) {
  const canAccept = status === "ready" && draft !== null && !accepting;
  // A draft the DATA check could not fill is still a valid spec: the builder is
  // where a reviewer fixes a window or a filter by hand, so it stays reachable.
  // A draft the registry refused has no spec worth loading, so edit stays shut
  // and the copy must not offer it.
  const canEdit =
    (status === "ready" || status === "accepted" || status === "needs_fixes") &&
    draft !== null &&
    issuesOutsideVerification(issues, verification).length === 0;
  const failed = status === "needs_fixes" || status === "refused";
  const title = draft?.name ?? (name.trim() === "" ? "Untitled report" : name.trim());
  const registryIssues = issuesOutsideVerification(issues, verification);

  return (
    <Card
      className="animate-[m-rise_120ms_cubic-bezier(0.2,0,0,1)] overflow-hidden motion-reduce:animate-none"
      data-testid="report-draft-row"
      data-status={status}
    >
      <CardHeader className="flex flex-wrap items-start justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Icon name="chart" size={16} />
            <h2 className="min-w-0 truncate text-[15px] font-semibold">{title}</h2>
            <Pill tone={DRAFT_STATUS_TONE[status]} dot>
              {DRAFT_STATUS_LABEL[status]}
            </Pill>
            {diff.length > 0 ? (
              <span className="font-mono text-[12px] tabular-nums text-muted-foreground">
                {diffTally(diff)}
              </span>
            ) : null}
          </div>
          {/* The plain-English read-back is the server's, derived from the spec it
              validated. This component never writes that sentence. */}
          {draft !== null ? (
            <p className="mt-1.5 max-w-[68ch] text-[13px] leading-[1.5] text-muted-foreground">
              {draft.reads}
            </p>
          ) : null}
          {status === "refused" && refusalMessage ? (
            <p role="alert" className="mt-1.5 text-[13px] font-medium text-mc-st-breach-fg">
              {refusalMessage}
            </p>
          ) : null}
        </div>

        {draft !== null ? (
          <dl className="flex shrink-0 flex-wrap items-baseline gap-x-4 gap-y-1 text-[12px]">
            <Fact label="Shape" value={draft.shape} />
            <Fact label="Window" value={`${draft.card.window_days}d`} />
            <Fact label="Refresh" value={`${draft.card.refresh?.every_minutes ?? 60}m`} />
            <Fact
              label="Size"
              value={`${draft.card.size?.span ?? "half"} / ${draft.card.size?.height ?? "regular"}`}
            />
          </dl>
        ) : null}
      </CardHeader>

      {verification !== null ? (
        <CardContent className="grid gap-2">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <h3 className="text-[11px] font-semibold uppercase tracking-[.06em] text-muted-foreground">
              What the check found
            </h3>
            <span
              className="font-mono text-[12px] tabular-nums text-muted-foreground"
              data-testid="draft-verification-summary"
            >
              {verificationSummary(verification)}
            </span>
          </div>

          {verification.ran && verification.checked.length > 0 ? (
            <details className="group">
              <summary className="cursor-pointer text-[12px] text-muted-foreground">
                What was checked
              </summary>
              <ul className="mt-1 grid gap-1">
                {verification.checked.map((check) => (
                  <li
                    key={check}
                    className="grid grid-cols-[14px_minmax(0,1fr)] items-baseline gap-2 text-[12px] leading-[1.5] text-muted-foreground"
                  >
                    <span className="text-mc-green-text">
                      <Icon name="check" size={13} />
                    </span>
                    <span className="min-w-0">{check}</span>
                  </li>
                ))}
              </ul>
            </details>
          ) : null}

          {verification.repaired.length > 0 ? (
            <FindingList
              title="Repaired before this draft was offered"
              findings={verification.repaired}
              tone="repaired"
            />
          ) : null}

          {verification.findings.length > 0 ? (
            <FindingList
              title="What the data could not fill"
              findings={verification.findings}
              tone="failing"
            />
          ) : null}
        </CardContent>
      ) : null}

      {registryIssues.length > 0 ? (
        <CardContent className="grid gap-2">
          <h3 className="text-[11px] font-semibold uppercase tracking-[.06em] text-muted-foreground">
            What the registry refused
          </h3>
          <ul className="grid gap-1.5">
            {registryIssues.map((issue) => (
              <li
                key={`${issue.code}:${issue.path ?? ""}:${issue.message}`}
                className="grid grid-cols-[16px_minmax(0,1fr)] items-baseline gap-2 text-[13px] leading-[1.5]"
              >
                <span className="text-mc-attn-fg"><Icon name="alert" size={14} /></span>
                <span className="min-w-0">
                  <span className="font-mono text-[12px] text-muted-foreground">{issue.code}</span>
                  {issue.path ? (
                    <>
                      {" · "}
                      <span className="font-medium">{issue.path}</span>
                    </>
                  ) : null}
                  <span className="block text-muted-foreground">{issue.message}</span>
                </span>
              </li>
            ))}
          </ul>
          <p className="text-[12px] text-muted-foreground">
            Nothing was saved. Reword the transcript and generate again.
          </p>
        </CardContent>
      ) : null}

      {draft !== null && diff.length > 0 ? (
        <CardContent>
          <DiffList diff={diff} />
        </CardContent>
      ) : null}

      {acceptError !== null ? (
        <CardContent>
          <Banner icon={<Icon name="alert" size={14} />}>{acceptError}</Banner>
        </CardContent>
      ) : null}

      <CardContent className="flex flex-wrap items-end gap-3">
        {status !== "refused" && status !== "accepted" ? (
          <label className="grid min-w-[220px] flex-1 gap-1">
            <span className="text-[11px] font-semibold uppercase tracking-[.06em] text-muted-foreground">
              Save as
            </span>
            <Input
              value={name}
              onChange={(event) => onNameChange(event.target.value)}
              placeholder="Name this report"
              aria-label="Report name"
            />
          </label>
        ) : null}

        <div className="ml-auto flex flex-wrap items-center gap-2">
          {/* A saved draft is no longer a proposal: there is nothing left to
              accept, and "reject" would claim to undo a row that exists. */}
          {status !== "accepted" ? (
            <>
              <Button variant="ghost" onClick={onReject} disabled={accepting}>
                {status === "refused" ? "Dismiss" : "Reject"}
              </Button>
              <Button variant="secondary" onClick={onEdit} disabled={!canEdit || accepting}>
                Edit in the builder
              </Button>
              <Button variant="primary" onClick={onAccept} disabled={!canAccept}>
                {accepting ? <Spinner /> : null} Accept and save
              </Button>
            </>
          ) : (
            <Button variant="secondary" onClick={onEdit}>
              Edit in the builder
            </Button>
          )}
        </div>
      </CardContent>

      {failed && draft === null ? (
        <CardContent>
          <p className="text-[12px] text-muted-foreground">
            Accept is off because there is no validated draft to save. The proposal
            above says why.
          </p>
        </CardContent>
      ) : null}
    </Card>
  );
}

/**
 * One class of finding, as the reviewer reads it: the machine class in mono,
 * then the sentence. Repaired findings are muted because they are history;
 * surviving ones carry attention because they are why Accept is off.
 */
function FindingList({
  title,
  findings,
  tone,
}: {
  title: string;
  findings: DraftFinding[];
  tone: "repaired" | "failing";
}) {
  return (
    <div className="grid gap-1.5">
      <h4 className="text-[11px] font-semibold uppercase tracking-[.06em] text-muted-foreground">
        {title}
      </h4>
      <ul className="grid gap-1.5">
        {findings.map((finding) => (
          <li
            key={`${finding.kind}:${finding.card_index ?? ""}:${finding.message}`}
            className="grid grid-cols-[16px_minmax(0,1fr)] items-baseline gap-2 text-[13px] leading-[1.5]"
          >
            <span className={tone === "failing" ? "text-mc-attn-fg" : "text-mc-green-text"}>
              <Icon name={tone === "failing" ? "alert" : "check"} size={14} />
            </span>
            <span className="min-w-0">
              <span className="font-mono text-[12px] text-muted-foreground">{finding.kind}</span>
              {finding.card_index !== undefined ? (
                <span className="font-mono text-[12px] text-muted-foreground">
                  {" · card "}
                  {finding.card_index + 1}
                </span>
              ) : null}
              <span
                className={
                  tone === "failing"
                    ? "block text-foreground"
                    : "block text-muted-foreground"
                }
              >
                {finding.message}
              </span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** One labelled fact: the label above the value, in the same position every time. */
function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid gap-0.5">
      <dt className="text-[10px] font-semibold uppercase tracking-[.08em] text-muted-foreground">
        {label}
      </dt>
      <dd className="font-mono text-[12px] text-foreground">{value}</dd>
    </div>
  );
}
