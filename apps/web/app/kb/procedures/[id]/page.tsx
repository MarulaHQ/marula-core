"use client";
// /kb/procedures/[id] — one runbook: ordered steps, the body, the owner.
//
// Purpose: the pack's process shape. A procedure is read top to bottom, so the
// steps are numbered and the required ones say so; the body is the prose that
// explains why. Ticking a step is deliberately not persisted here — a template
// is not a run record, and inventing a run store for it would duplicate the
// workflow engine.

import Link from "next/link";
import { useParams } from "next/navigation";
import { useEffect, useState } from "react";
import { useRequireAuth } from "../../../../lib/api";
import { SurfaceHeader } from "@marula-core/ui";
import {
  ErrorNote,
  PageFrame,
  SkeletonRows,
  StatusPill,
} from "../../../../components/knowledge/ledger";
import {
  knowledgeApi,
  type KnowledgeRequestError,
  type ProcedureDetail,
} from "../../../../components/knowledge/knowledge-api";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  SummaryCard,
} from "../../../../components/records/record-frame";

export default function ProcedurePage() {
  useRequireAuth();
  const params = useParams<{ id: string }>();
  const procedureId = params.id;
  const [procedure, setProcedure] = useState<ProcedureDetail | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (procedureId === undefined) return;
    let cancelled = false;
    knowledgeApi
      .get<ProcedureDetail>(`/api/v1/knowledge/procedures/${procedureId}`)
      .then((data) => {
        if (!cancelled) setProcedure(data);
      })
      .catch((e: KnowledgeRequestError) => {
        if (!cancelled) setError(e.message);
      });
    return () => {
      cancelled = true;
    };
  }, [procedureId]);

  if (error !== null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <ErrorNote title="The procedure did not load.">{error}</ErrorNote>
        </div>
      </PageFrame>
    );
  }

  if (procedure === null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <SkeletonRows rows={7} sentence="Loading the procedure" columns={3} />
        </div>
      </PageFrame>
    );
  }

  return (
    <PageFrame>
      <div className="flex flex-col gap-3 pb-4 pt-5">
        <nav aria-label="Breadcrumb" className="flex items-center gap-1.5 text-[13px]">
          <Link href="/kb" className="underline underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
            Knowledge
          </Link>
          <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
            /
          </span>
          <Link
            href={`/kb/procedures?client_id=${procedure.client_id}`}
            className="underline underline-offset-2"
            style={{ color: "var(--m-text-secondary)" }}
          >
            Procedures
          </Link>
          <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
            /
          </span>
          <span className="font-semibold">{procedure.title}</span>
        </nav>

        <RecordFrame
          context={
            <>
              <RecordSection label="Procedure">
                <p className="text-[18px] font-semibold leading-tight">{procedure.title}</p>
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {procedure.summary ?? "No summary"}
                </p>
              </RecordSection>
              <RecordSection label="Ownership">
                <RecordRow label="Owner">{procedure.owner_name ?? "Unassigned"}</RecordRow>
                <RecordRow label="Version">v{procedure.version}</RecordRow>
                <RecordRow label="Visibility">
                  {procedure.client_visible ? "Client can read it" : "Internal only"}
                </RecordRow>
              </RecordSection>
              <RecordSection label="Shape">
                <RecordRow label="Steps">{procedure.steps.length}</RecordRow>
                <RecordRow label="Checklist">{procedure.is_checklist ? "Yes" : "No"}</RecordRow>
              </RecordSection>
            </>
          }
          content={
            <div className="flex flex-col gap-4 pt-2">
<SurfaceHeader
          title={procedure.title}
          meta={<StatusPill tone="neutral">v{procedure.version}</StatusPill>}
        />

              {procedure.steps.length > 0 ? (
                <section aria-label="Steps" className="flex flex-col gap-2">
                  <h2 className="m-0 text-[15px] font-semibold">Steps</h2>
                  <ol className="m-0 flex list-none flex-col gap-0 p-0">
                    {procedure.steps.map((step) => (
                      <li
                        key={step.id}
                        className="flex gap-3 py-3"
                        style={{ borderTop: "1px solid var(--m-border)" }}
                      >
                        <span
                          aria-hidden="true"
                          className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[12px] font-bold tabular-nums"
                          style={{ background: "var(--m-surface-2)" }}
                        >
                          {step.position + 1}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block text-[14px] font-semibold">{step.title}</span>
                          {step.body !== null && step.body !== "" ? (
                            <span
                              className="mt-1 block whitespace-pre-wrap text-[13px] leading-relaxed"
                              style={{ color: "var(--m-text-secondary)" }}
                            >
                              {step.body}
                            </span>
                          ) : null}
                          {!step.required ? (
                            <span className="mt-1 block text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                              Optional step
                            </span>
                          ) : null}
                        </span>
                      </li>
                    ))}
                  </ol>
                </section>
              ) : null}

              <section aria-label="Runbook">
                <h2 className="m-0 mb-2 text-[15px] font-semibold">Runbook</h2>
                <div
                  className="whitespace-pre-wrap rounded-[16px] border p-4 text-[14px] leading-relaxed"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                >
                  {procedure.body !== ""
                    ? procedure.body
                    : "No body yet. The steps above are the procedure; the body explains why they are in that order."}
                </div>
              </section>
            </div>
          }
          summary={
            <SummaryCard title="Linked records">
              {procedure.links.length === 0 ? (
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Nothing linked. Link the assets and articles this runbook touches.
                </p>
              ) : (
                procedure.links.map((link) => (
                  <RecordRow key={link.id} label={link.label ?? link.to_kind}>
                    {link.to_kind}
                  </RecordRow>
                ))
              )}
            </SummaryCard>
          }
        />
      </div>
    </PageFrame>
  );
}
