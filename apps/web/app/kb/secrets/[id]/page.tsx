"use client";
// /kb/secrets/[id] — one credential: masked, revealed on purpose, auto-hidden.
//
// Purpose: the pack's K08, K10 and K16 in one page. The value is masked by
// default; Reveal is an audited act that returns the plaintext once and starts
// a 30-second countdown that hides it again. A two-step code rides the same
// protection: the seed is sealed like the password, and the digits are produced
// only by an audited read.
//
// Non-purpose: no editing and no sharing — both already exist on the vault
// page, and a second editor here would be a second source of truth.

import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, useRequireAuth } from "../../../../lib/api";
import { SurfaceHeader } from "@marula-core/ui";
import {
  ErrorNote,
  LockIcon,
  PageFrame,
  SkeletonRows,
  StatusPill,
} from "../../../../components/knowledge/ledger";
import {
  knowledgeApi,
  type KnowledgeRequestError,
} from "../../../../components/knowledge/knowledge-api";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  SummaryCard,
} from "../../../../components/records/record-frame";

interface SecretRecord {
  id: string;
  title: string;
  kind: string;
  client_id: string | null;
  expires_at: string | null;
  rotate_at: string | null;
  updated_at: string;
}

interface RevealField {
  field_key: string;
  value: string;
}

interface Reveal {
  id: string;
  title: string;
  fields: RevealField[];
}

interface TotpState {
  record_id: string;
  configured: boolean;
  digits: number | null;
  period_seconds: number | null;
  algorithm: string | null;
  issuer: string | null;
  account: string | null;
  created_at: string | null;
}

interface TotpCode {
  code: string;
  digits: number;
  period_seconds: number;
  seconds_remaining: number;
}

const FIELD_LABEL: Record<string, string> = {
  secret: "Password",
  username: "Username",
  url: "Address",
  notes: "Notes",
};

export default function SecretPage() {
  useRequireAuth();
  const params = useParams<{ id: string }>();
  const recordId = params.id;
  const [record, setRecord] = useState<SecretRecord | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [revealError, setRevealError] = useState<string | null>(null);
  const [revealed, setRevealed] = useState<Reveal | null>(null);
  const [secondsLeft, setSecondsLeft] = useState<number>(0);
  const [totp, setTotp] = useState<TotpState | null>(null);
  const [code, setCode] = useState<TotpCode | null>(null);
  const [codeLeft, setCodeLeft] = useState(0);
  const [totpError, setTotpError] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const hideTimer = useRef<number | null>(null);

  const hide = useCallback(() => {
    setRevealed(null);
    setSecondsLeft(0);
    setCode(null);
    setCodeLeft(0);
    if (hideTimer.current !== null) {
      window.clearInterval(hideTimer.current);
      hideTimer.current = null;
    }
  }, []);

  useEffect(() => {
    if (recordId === undefined) return;
    let cancelled = false;
    setError(null);
    api.api.GET("/api/v1/secrets/records/{id}", { params: { path: { id: recordId } } }).then(({ data, error: apiError }) => {
      if (cancelled) return;
      if (apiError !== undefined || data === undefined) {
        setError("This credential is not visible to you, or it no longer exists.");
        return;
      }
      setRecord(data as unknown as SecretRecord);
    });
    knowledgeApi
      .get<TotpState>(`/api/v1/knowledge/secrets/${recordId}/totp`)
      .then((state) => {
        if (!cancelled) setTotp(state);
      })
      .catch(() => {
        // A missing two-step configuration is not an error the reader needs.
        if (!cancelled) setTotp(null);
      });
    return () => {
      cancelled = true;
    };
  }, [recordId]);

  useEffect(() => hide, [hide]);

  async function reveal() {
    if (recordId === undefined) return;
    setRevealError(null);
    const { data, error: apiError } = await api.api.POST("/api/v1/secrets/records/{id}/reveal", {
      params: { path: { id: recordId } },
    });
    if (apiError !== undefined || data === undefined) {
      setRevealError("The reveal was refused. It needs vault edit permission, and a grant on this category.");
      return;
    }
    const payload = data as unknown as Reveal;
    setRevealed(payload);
    setSecondsLeft(30);
    if (hideTimer.current !== null) window.clearInterval(hideTimer.current);
    hideTimer.current = window.setInterval(() => {
      setSecondsLeft((current) => {
        if (current <= 1) {
          hide();
          return 0;
        }
        return current - 1;
      });
    }, 1000);
  }

  async function readCode() {
    if (recordId === undefined) return;
    setTotpError(null);
    try {
      const payload = await knowledgeApi.post<TotpCode>(
        `/api/v1/knowledge/secrets/${recordId}/totp/code`,
      );
      setCode(payload);
      setCodeLeft(payload.seconds_remaining);
      window.setTimeout(() => {
        setCodeLeft((current) => Math.max(0, current - 1));
      }, 1000);
      const interval = window.setInterval(() => {
        setCodeLeft((current) => {
          if (current <= 1) {
            window.clearInterval(interval);
            setCode(null);
            return 0;
          }
          return current - 1;
        });
      }, 1000);
    } catch (e) {
      setTotpError((e as KnowledgeRequestError).message);
    }
  }

  async function attachCode(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (recordId === undefined) return;
    const form = event.currentTarget;
    const data = new FormData(form);
    setTotpError(null);
    setAdding(false);
    try {
      const state = await knowledgeApi.post<TotpState>(
        `/api/v1/knowledge/secrets/${recordId}/totp`,
        {
          seed_base32: String(data.get("seed") ?? ""),
          digits: Number(data.get("digits") ?? 6),
          verify_code: String(data.get("verify") ?? ""),
          issuer: "Marula",
        },
      );
      setTotp(state);
      form.reset();
    } catch (e) {
      setTotpError((e as KnowledgeRequestError).message);
    }
  }

  if (error !== null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <ErrorNote title="The credential did not load.">{error}</ErrorNote>
        </div>
      </PageFrame>
    );
  }

  if (record === null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <SkeletonRows rows={6} sentence="Loading the credential" columns={3} />
        </div>
      </PageFrame>
    );
  }

  const passwordField = revealed?.fields.find((field) => field.field_key === "secret") ?? null;
  const otherFields = revealed?.fields.filter((field) => field.field_key !== "secret") ?? [];

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
          <Link href="/kb/secrets" className="underline underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
            Vault
          </Link>
          <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
            /
          </span>
          <span className="font-semibold">{record.title}</span>
        </nav>

        <RecordFrame
          context={
            <>
              <RecordSection label="Credential">
                <p className="text-[18px] font-semibold leading-tight">{record.title}</p>
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {record.kind}
                </p>
              </RecordSection>
              <RecordSection label="Scope">
                <RecordRow label="Client">
                  {record.client_id !== null ? (
                    <Link href={`/kb/clients/${record.client_id}`} className="underline underline-offset-2">
                      Open client
                    </Link>
                  ) : (
                    "Org-level"
                  )}
                </RecordRow>
                <RecordRow label="Expires">{record.expires_at?.slice(0, 10) ?? "Never"}</RecordRow>
                <RecordRow label="Rotation">
                  {record.rotate_at?.slice(0, 10) ?? "Not scheduled"}
                </RecordRow>
              </RecordSection>
              <RecordSection label="Audit">
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  Every reveal and copy is written to the audit trail with who, when and why. The
                  trail is append-only; nobody, including an admin, can edit it.
                </p>
              </RecordSection>
            </>
          }
          content={
            <div className="flex flex-col gap-4 pt-2">
<SurfaceHeader
          title={record.title}
          meta={
            <>
              <StatusPill tone="neutral">{record.kind}</StatusPill>
              {totp?.configured === true ? <StatusPill tone="success">Two-step code</StatusPill> : null}
            </>
          }
        />

              {revealed !== null ? (
                <div
                  role="status"
                  className="flex items-center gap-2 rounded-[12px] px-3.5 py-2.5 text-[13px] font-medium"
                  style={{ background: "var(--m-status-success-bg)", color: "var(--m-status-success-fg)" }}
                >
                  <LockIcon size={16} />
                  Revealed and logged. It hides again by itself in {secondsLeft}s.
                </div>
              ) : null}
              {revealError !== null ? <ErrorNote title="Reveal refused.">{revealError}</ErrorNote> : null}

              <section
                className="flex flex-col gap-3 rounded-[16px] border p-4"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                aria-label="Credentials"
              >
                <div className="flex flex-wrap items-center gap-3">
                  <span className="w-[110px] text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                    Password
                  </span>
                  {passwordField !== null ? (
                    <code
                      className="rounded-[8px] px-2 py-1 text-[15px]"
                      style={{ background: "var(--m-surface-2)", fontFamily: "var(--m-font-mono)" }}
                    >
                      {passwordField.value}
                    </code>
                  ) : (
                    <span
                      className="text-[16px] tracking-[0.12em]"
                      style={{ fontFamily: "var(--m-font-mono)" }}
                      aria-label="Password hidden"
                    >
                      ••••••••••••
                    </span>
                  )}
                  <span className="flex-1" />
                  {revealed === null ? (
                    <button
                      type="button"
                      onClick={reveal}
                      className="h-11 rounded-[10px] px-4 text-[13px] font-semibold"
                      style={{ background: "var(--m-ink)", color: "var(--m-gold)" }}
                    >
                      Reveal
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={hide}
                      className="h-11 rounded-[10px] border px-4 text-[13px] font-semibold"
                      style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                    >
                      Hide now
                    </button>
                  )}
                </div>
                {otherFields.map((field) => (
                  <div key={field.field_key} className="flex flex-wrap items-center gap-3">
                    <span className="w-[110px] text-[13px]" style={{ color: "var(--m-text-muted)" }}>
                      {FIELD_LABEL[field.field_key] ?? field.field_key}
                    </span>
                    <span className="text-[13px]">{field.value}</span>
                  </div>
                ))}
              </section>

              <section
                className="flex flex-col gap-3 rounded-[16px] border p-4"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                aria-label="Two-step code"
              >
                <div className="flex flex-wrap items-center gap-2">
                  <h2 className="m-0 text-[15px] font-semibold">Two-step code</h2>
                  <span className="flex-1" />
                  {totp?.configured === true ? (
                    <button
                      type="button"
                      onClick={readCode}
                      className="h-9 rounded-[10px] px-3 text-[13px] font-semibold"
                      style={{ background: "var(--m-ink)", color: "var(--m-gold)" }}
                    >
                      Show code
                    </button>
                  ) : (
                    <button
                      type="button"
                      onClick={() => setAdding((open) => !open)}
                      className="h-9 rounded-[10px] border px-3 text-[13px] font-semibold"
                      style={{ borderColor: "var(--m-border)" }}
                    >
                      Add code
                    </button>
                  )}
                </div>

                {totpError !== null ? <ErrorNote title="The code did not land.">{totpError}</ErrorNote> : null}

                {totp?.configured !== true ? (
                  <p className="m-0 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                    No authenticator attached. The seed is stored sealed with the same key as the
                    password and is never shown again after setup.
                  </p>
                ) : code !== null ? (
                  <div className="flex flex-wrap items-center gap-3">
                    <code
                      className="text-[26px] tracking-[0.12em]"
                      style={{ fontFamily: "var(--m-font-mono)" }}
                    >
                      {code.code}
                    </code>
                    <span className="text-[13px] tabular-nums" style={{ color: "var(--m-text-secondary)" }}>
                      New code in {codeLeft}s
                    </span>
                  </div>
                ) : (
                  <p className="m-0 text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                    {totp.digits} digits, every {totp.period_seconds} seconds. Showing it is audited
                    exactly like a reveal.
                  </p>
                )}

                {adding && totp?.configured !== true ? (
                  <form onSubmit={attachCode} className="flex flex-col gap-2" aria-label="Add a two-step code">
                    <label className="flex flex-col gap-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                      <span>Setup key from the authenticator</span>
                      <input
                        name="seed"
                        required
                        autoComplete="off"
                        className="h-10 rounded-[10px] border px-2.5 text-[13px]"
                        style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                      />
                    </label>
                    <div className="flex flex-wrap items-end gap-2">
                      <label className="flex flex-col gap-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                        <span>Digits</span>
                        <select
                          name="digits"
                          defaultValue={6}
                          className="h-10 rounded-[10px] border px-2.5 text-[13px]"
                          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                        >
                          <option value={6}>6</option>
                          <option value={8}>8</option>
                        </select>
                      </label>
                      <label className="flex flex-col gap-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                        <span>Code it shows now</span>
                        <input
                          name="verify"
                          required
                          inputMode="numeric"
                          autoComplete="one-time-code"
                          className="h-10 w-32 rounded-[10px] border px-2.5 text-[13px] tracking-[0.2em]"
                          style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                        />
                      </label>
                      <button
                        type="submit"
                        className="h-10 rounded-[10px] px-4 text-[13px] font-semibold"
                        style={{ background: "var(--m-ink)", color: "var(--m-gold)" }}
                      >
                        Check and save
                      </button>
                    </div>
                    <p className="m-0 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                      The code must match before the seed is saved — a seed nobody checked is a seed
                      that does not work at 3am.
                    </p>
                  </form>
                ) : null}
              </section>
            </div>
          }
          summary={
            <>
              <SummaryCard title="Protection">
                <p className="text-[13px] leading-relaxed" style={{ color: "var(--m-text-secondary)" }}>
                  Reveal needs vault edit permission and a grant on this category. The value hides
                  itself again after 30 seconds, and every read is logged.
                </p>
              </SummaryCard>
              <SummaryCard title="Rotation">
                <RecordRow label="Last updated">{record.updated_at.slice(0, 10)}</RecordRow>
                <RecordRow label="Scheduled">{record.rotate_at?.slice(0, 10) ?? "Not scheduled"}</RecordRow>
              </SummaryCard>
            </>
          }
        />
      </div>
    </PageFrame>
  );
}
