// /settings/email/templates/[id] — the admin email template editor.
//
// Purpose: edit one template's subject and rich-text HTML body (Tiptap,
// StarterKit only — the bundle stays honest), with a variable-insertion
// dropdown drawn from the kind's closed vocabulary, an auto-derived
// plain-text fallback view, and a sample-data preview pane that never
// autosaves and never sends.
//
// Save semantics (instant-first): debounced autosave on every editable
// field through the IF1/IF2 envelope (idempotency key + version echo),
// exactly like the KB editor. A draft that would fail the server's
// write-time checks (empty name/body, an out-of-vocabulary placeholder) is
// blocked locally with the reason shown, so autosave never loops on 400s.
//
// Shipped defaults (`is_system`) are immutable at the API: the page renders
// them read-only with a duplicate-to-edit affordance instead of a dead
// editor.
//
// Known API gaps, kept honest rather than papered over (TODOs in the lane
// report): PATCH cannot CLEAR a subject (absent = keep current), and no
// stored plain-text field exists — the fallback view is derived, marked as
// such, and read-only.
"use client";

import Link from "next/link";
import { useParams, useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardHeader,
  FieldError,
  FieldHint,
  Input,
  KbProse,
  Spinner,
  Textarea,
} from "@marula-core/ui";
import { api, useRequireAuth } from "../../../../../lib/api";
import { runEnvelopeMutation, rememberVersion } from "../../../../../lib/envelope-write";
import { EmailTemplateEditorLazy } from "../../../../../lib/editor/email-template-editor-lazy";
import { SettingsHeader } from "../../../../../components/settings/settings-shell";
import { ForkDialog } from "../../../../../components/settings/email-templates/fork-dialog";
import { VariableSelect } from "../../../../../components/settings/email-templates/variable-select";
import { extractPlaceholders, htmlToPlainText, renderDraft } from "../../../../../lib/email-templates/derive";
import { SAMPLE_CONTEXT } from "../../../../../lib/email-templates/sample-context";
import { templateKindLabel, variableLabel } from "../../../../../lib/email-templates/variables";
import type {
  TemplateMutationResult,
  TemplateRow,
  TemplatesList,
  UpdateTemplateBody,
} from "../../../../../lib/email-templates/types";

const AUTOSAVE_MS = 800;

interface Draft {
  name: string;
  description: string;
  subject: string;
  body: string;
}

/**
 * Why this draft cannot be autosaved yet. Mirrors the server's write-time
 * checks (routes/templates.rs) so a blocked save never becomes a 400 loop.
 * A null vocabulary means the kind's list has not loaded — the placeholder
 * check waits for it rather than guessing, and the server stays the final
 * word.
 */
function draftProblem(draft: Draft, vocabulary: string[] | null): string | null {
  if (draft.name.trim() === "") return "The template needs a name before it can save.";
  if (draft.body.trim() === "") return "The body can't be empty.";
  if (vocabulary === null) return null;
  for (const key of extractPlaceholders(`${draft.subject}\n${draft.body}`)) {
    if (!vocabulary.includes(key)) {
      return `{{${key}}} is not in the variable list for this kind — remove it or save once the list includes it.`;
    }
  }
  return null;
}

export default function TemplateEditorPage() {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  const router = useRouter();
  const [template, setTemplate] = useState<TemplateRow | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [vocabulary, setVocabulary] = useState<string[] | null>(null);

  useEffect(() => {
    if (auth.status !== "authed") return;
    let cancelled = false;
    void (async () => {
      const res = await api.api.GET("/api/v1/templates/{id}", {
        params: { path: { id: params.id } },
      });
      if (cancelled) return;
      if (res.error || !res.data) {
        setLoadError("This template could not be loaded. It may have been deleted.");
        return;
      }
      setTemplate(res.data as unknown as TemplateRow);
    })();
    return () => {
      cancelled = true;
    };
  }, [auth.status, params.id]);

  // The per-kind vocabulary rides the templates list response (one cheap
  // call, limit 1). If it fails, the editor runs with an empty list: the
  // dropdown hides rather than guesses, and the server's write-time check
  // stays the final word.
  useEffect(() => {
    if (!template) return;
    let cancelled = false;
    void (async () => {
      const res = await api.api.GET("/api/v1/templates", {
        params: { query: { kind: template.kind, limit: 1 } },
      });
      if (cancelled) return;
      const list = res.data as unknown as TemplatesList | undefined;
      setVocabulary(list?.vocabulary.find((v) => v.kind === template.kind)?.variables ?? []);
    })();
    return () => {
      cancelled = true;
    };
  }, [template]);

  if (auth.status !== "authed") return null;

  if (loadError || (!template && !loadError)) {
    return (
      <div className="grid gap-4">
        <SettingsHeader title="Email templates" description="Loading the template…" />
        {loadError ? (
          <div className="grid gap-2">
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
              {loadError}
            </p>
            <Button variant="secondary" asChild>
              <Link href="/settings/email/templates">Back to templates</Link>
            </Button>
          </div>
        ) : (
          <Spinner />
        )}
      </div>
    );
  }

  const templateRow = template;
  if (!templateRow) return null;

  const kindLabel = templateKindLabel(templateRow.kind);
  const provenance =
    templateRow.is_system && templateRow.builtin ? (
      <Badge tone="outline">Built-in</Badge>
    ) : templateRow.forked_from ? (
      <Badge tone="warning">Fork of {templateRow.forked_from}</Badge>
    ) : (
      <Badge tone="neutral">Custom</Badge>
    );

  return (
    <div className="grid gap-4">
      <Button variant="ghost" size="sm" asChild>
        <Link href="/settings/email/templates">← All templates</Link>
      </Button>
      <SettingsHeader title={templateRow.name} description={`${kindLabel} · ${templateRow.slug}`}>
        {provenance}
      </SettingsHeader>

      {templateRow.is_system ? (
        <ReadOnlyTemplate template={templateRow} vocabulary={vocabulary} />
      ) : (
        <TemplateEditor
          key={templateRow.id}
          template={templateRow}
          vocabulary={vocabulary}
          onForked={(id) => router.push(`/settings/email/templates/${id}`)}
        />
      )}
    </div>
  );
}

// --- read-only view (shipped defaults) ---------------------------------------

function ReadOnlyTemplate({
  template,
  vocabulary,
}: {
  template: TemplateRow;
  vocabulary: string[] | null;
}): React.JSX.Element {
  const [forkOpen, setForkOpen] = useState(false);
  const router = useRouter();

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_380px]">
      <div className="grid gap-4 content-start">
        <div
          role="status"
          className="rounded-md border px-3 py-2 text-[13px] flex flex-wrap items-center gap-3"
          style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)" }}
        >
          <span>
            Built-in templates are shipped defaults and can't be edited. Duplicate
            this one to make a copy your workspace can change.
          </span>
          <Button variant="primary" size="sm" onClick={() => setForkOpen(true)}>
            Duplicate and edit
          </Button>
        </div>
        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold">Subject</h2>
          </CardHeader>
          <CardContent>
            <p className="text-[13px]">{template.subject ?? "No subject — the template is body-only."}</p>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <h2 className="text-sm font-semibold">Body</h2>
          </CardHeader>
          <CardContent>
            <PreviewBody html={template.body} mode="html" />
          </CardContent>
        </Card>
      </div>
      <div className="grid gap-4 content-start">
        <PreviewPane
          subject={template.subject ?? ""}
          body={template.body}
          plainText={htmlToPlainText(template.body)}
          readOnly
        />
      </div>
      {forkOpen ? (
        <ForkDialog
          row={template}
          onClose={() => setForkOpen(false)}
          onForked={() => {}}
          onOpenCopy={(id) => router.push(`/settings/email/templates/${id}`)}
        />
      ) : null}
    </div>
  );
}

// --- editable editor ----------------------------------------------------------

function TemplateEditor({
  template,
  vocabulary,
  onForked,
}: {
  template: TemplateRow;
  vocabulary: string[] | null;
  onForked: (id: string) => void;
}): React.JSX.Element {
  const kindVocabulary = vocabulary ?? [];

  const [name, setName] = useState(template.name);
  const [description, setDescription] = useState(template.description);
  const [subject, setSubject] = useState(template.subject ?? "");
  const [bodyHtml, setBodyHtml] = useState(template.body);
  const [plainText, setPlainText] = useState(() => htmlToPlainText(template.body));
  const [status, setStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [error, setError] = useState<string | null>(null);

  // The debounce machinery never reads React state — refs carry the latest
  // draft so a burst of edits coalesces into one correct write.
  const latestRef = useRef<Draft>({ name, description, subject, body: template.body });
  const versionRef = useRef<number | null>(null);
  const inFlightRef = useRef(false);
  const dirtyRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const subjectInputRef = useRef<HTMLInputElement | null>(null);
  const subjectCaretRef = useRef<number | null>(null);

  const vocabularyRef = useRef<string[] | null>(vocabulary);
  vocabularyRef.current = vocabulary;

  const flush = useCallback(async () => {
    if (inFlightRef.current || !dirtyRef.current) return;
    const draft = latestRef.current;
    const problem = draftProblem(draft, vocabularyRef.current);
    if (problem !== null) {
      setStatus("error");
      setError(problem);
      return;
    }
    inFlightRef.current = true;
    dirtyRef.current = false;
    setStatus("saving");
    setError(null);
    const entityId = template.id;
    const result = await runEnvelopeMutation<UpdateTemplateBody, TemplateMutationResult>({
      describe: "Template autosave",
      entityId,
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        ...(versionRef.current !== null ? { version: versionRef.current } : {}),
        name: draft.name,
        description: draft.description,
        // An empty subject field is NOT sent: the API reads an absent
        // subject as "keep the current one" (PATCH cannot clear it), so
        // sending a value would misrepresent what the draft holds.
        ...(draft.subject.trim() === "" ? {} : { subject: draft.subject }),
        body: draft.body,
        variables: extractPlaceholders(`${draft.subject}\n${draft.body}`).map((key) => ({
          key,
          label: variableLabel(key),
        })),
      }),
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client
          .PATCH("/api/v1/templates/{id}", {
            params: { path: { id: entityId } },
            // The generated UpdateTemplateRequest schema is a name
            // collision (see lib/email-templates/types.ts) — cast through.
            body: body as never,
          })
          .then((r) => ({ ...r, data: r.data as unknown as TemplateMutationResult })),
      versions: (response) => {
        versionRef.current = response.mutation.version;
        rememberVersion(entityId, response.mutation.version);
        return [[entityId, response.mutation.version]];
      },
    });
    inFlightRef.current = false;
    if (!result.ok) {
      setStatus("error");
      setError(result.friendly ?? "The save was rejected.");
      if (result.reason === "version_conflict") {
        // The next save goes out version-less; the server resolves the
        // current revision instead of bouncing the user through a reload.
        versionRef.current = null;
      }
      return;
    }
    setStatus("saved");
    if (dirtyRef.current) {
      dirtyRef.current = false;
      scheduleFlush();
    }
  }, [template.id]);

  const scheduleFlush = useCallback(() => {
    if (inFlightRef.current) {
      dirtyRef.current = true;
      return;
    }
    if (timerRef.current !== null) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void flush();
    }, AUTOSAVE_MS);
  }, [flush]);

  useEffect(() => {
    rememberVersion(template.id, null);
    return () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    };
  }, [template.id]);

  // The body editor is a lazy leaf (lib/editor/email-template-editor-lazy):
  // TipTap loads only when this page mounts, and every HTML change lands
  // here through the boundary's onHtmlChange — the exact write path the
  // inline useEditor's onUpdate used to drive.
  const handleBodyChange = useCallback(
    (html: string) => {
      latestRef.current = { ...latestRef.current, body: html };
      setBodyHtml(html);
      dirtyRef.current = true;
      setStatus("idle");
      scheduleFlush();
    },
    [scheduleFlush],
  );

  const patchDraft = (patch: Partial<Draft>) => {
    latestRef.current = { ...latestRef.current, ...patch };
    dirtyRef.current = true;
    setStatus("idle");
    scheduleFlush();
  };

  const insertIntoSubject = (key: string) => {
    const token = `{{${key}}}`;
    const caret = subjectCaretRef.current ?? subject.length;
    const next = subject.slice(0, caret) + token + subject.slice(caret);
    setSubject(next);
    patchDraft({ subject: next });
    requestAnimationFrame(() => {
      const input = subjectInputRef.current;
      if (input) {
        input.focus();
        const position = caret + token.length;
        input.setSelectionRange(position, position);
        subjectCaretRef.current = position;
      }
    });
  };

  const draft: Draft = { name, description, subject, body: bodyHtml };
  const problem = draftProblem(draft, vocabulary);

  return (
    <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_380px]">
      <div className="grid gap-4 content-start">
        <Card>
          <CardHeader>
            <div>
              <h2 className="text-sm font-semibold">Template</h2>
              <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                Changes save automatically a moment after you stop typing.
              </p>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid gap-4">
              <label className="grid gap-1">
                <span className="text-xs">Name</span>
                <Input
                  value={name}
                  onChange={(e) => {
                    setName(e.target.value);
                    patchDraft({ name: e.target.value });
                  }}
                  maxLength={128}
                  required
                />
              </label>
              <label className="grid gap-1">
                <span className="text-xs">Description</span>
                <Input
                  value={description}
                  onChange={(e) => {
                    setDescription(e.target.value);
                    patchDraft({ description: e.target.value });
                  }}
                  maxLength={200}
                  placeholder="Where this template is used, in one line"
                />
              </label>
              <div className="grid gap-1">
                <div className="flex items-end gap-2">
                  <label className="grid gap-1 flex-1 min-w-0">
                    <span className="text-xs">Subject</span>
                    <Input
                      ref={subjectInputRef}
                      value={subject}
                      onChange={(e) => {
                        setSubject(e.target.value);
                        subjectCaretRef.current = e.target.selectionStart;
                        patchDraft({ subject: e.target.value });
                      }}
                      onSelect={(e) => {
                        subjectCaretRef.current = e.currentTarget.selectionStart;
                      }}
                      placeholder="Ticket {{ticket.number}}: {{ticket.summary}}"
                      maxLength={998}
                    />
                  </label>
                  <VariableSelect
                    vocabulary={kindVocabulary}
                    onInsert={insertIntoSubject}
                    ariaLabel="Insert variable into subject"
                  />
                </div>
                <FieldHint>
                  Leave empty for a body-only template. An emptied subject stays at its
                  last saved value — the API keeps, rather than clears, an absent subject.
                </FieldHint>
              </div>
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <div>
              <h2 className="text-sm font-semibold">Body</h2>
              <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                The HTML email body. Bold, italic, underline, lists, headings and links.
              </p>
            </div>
          </CardHeader>
          <CardContent>
            <EmailTemplateEditorLazy
              initialContent={template.body}
              vocabulary={kindVocabulary}
              onHtmlChange={handleBodyChange}
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <div>
              <h2 className="text-sm font-semibold">Plain-text fallback</h2>
              <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
                What a plain-text mail client would read. Derived from the HTML body —
                this is a preview, not a second stored copy.
              </p>
            </div>
          </CardHeader>
          <CardContent>
            <div className="grid gap-2">
              <Textarea
                value={plainText}
                readOnly
                rows={8}
                aria-label="Plain-text fallback (derived from HTML body)"
                style={{ fontFamily: "var(--m-font-mono)" }}
              />
              <div className="flex items-center gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => setPlainText(htmlToPlainText(latestRef.current.body))}
                >
                  Derive from HTML body
                </Button>
                <span className="text-xs" style={{ color: "var(--m-text-muted)" }}>
                  Auto-derived — the API has no plain-text field to store edits yet.
                </span>
              </div>
            </div>
          </CardContent>
        </Card>
      </div>

      <div className="grid gap-4 content-start">
        <SaveStatus status={status} error={error} problem={problem} />
        <PreviewPane
          subject={subject}
          body={bodyHtml}
          plainText={renderDraft(htmlToPlainText(bodyHtml), SAMPLE_CONTEXT)}
          readOnly={false}
        />
      </div>
    </div>
  );
}

// --- shared pieces ------------------------------------------------------------

function SaveStatus({
  status,
  error,
  problem,
}: {
  status: "idle" | "saving" | "saved" | "error";
  error: string | null;
  problem: string | null;
}): React.JSX.Element {
  if (error !== null) {
    return (
      <p role="alert" className="text-xs" style={{ color: "var(--m-status-danger-fg)" }}>
        {error}
      </p>
    );
  }
  if (problem !== null) {
    return (
      <p role="status" className="text-xs" style={{ color: "var(--m-status-warning-fg)" }}>
        {problem}
      </p>
    );
  }
  if (status === "saving") {
    return (
      <p className="text-xs" style={{ color: "var(--m-text-secondary)" }} aria-live="polite">
        Saving…
      </p>
    );
  }
  if (status === "saved") {
    return (
      <p className="text-xs" style={{ color: "var(--m-status-success-fg)" }} aria-live="polite">
        Saved
      </p>
    );
  }
  return (
    <p className="text-xs" style={{ color: "var(--m-text-muted)" }}>
      Autosave on
    </p>
  );
}

function PreviewBody({ html, mode }: { html: string; mode: "html" | "text" }): React.JSX.Element {
  if (mode === "text") {
    return (
      <pre className="text-[13px] m-0 whitespace-pre-wrap" style={{ fontFamily: "var(--m-font-mono)" }}>
        {html}
      </pre>
    );
  }
  // The editor's own draft HTML — written by this editor's toolbar set
  // (no script, no unsanctioned embeds can serialize through it), previewed
  // locally, never fetched from an outside source.
  return <KbProse html={html} />;
}

function PreviewPane({
  subject,
  body,
  plainText,
  readOnly,
}: {
  subject: string;
  body: string;
  plainText: string;
  readOnly: boolean;
}): React.JSX.Element {
  const [mode, setMode] = useState<"html" | "text">("html");
  const previewSubject = renderDraft(subject, SAMPLE_CONTEXT);

  return (
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-2 w-full">
          <div>
            <h2 className="text-sm font-semibold">Preview</h2>
            <p className="mt-0.5 text-xs" style={{ color: "var(--m-text-secondary)" }}>
              {readOnly
                ? "The saved template with sample values."
                : "Your draft with sample values. Nothing here is saved or sent."}
            </p>
          </div>
          <div className="flex gap-1">
            <Button
              variant={mode === "html" ? "primary" : "secondary"}
              size="sm"
              aria-pressed={mode === "html"}
              onClick={() => setMode("html")}
            >
              HTML
            </Button>
            <Button
              variant={mode === "text" ? "primary" : "secondary"}
              size="sm"
              aria-pressed={mode === "text"}
              onClick={() => setMode("text")}
            >
              Text
            </Button>
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <div className="grid gap-3">
          <div
            role="status"
            className="rounded-md border px-3 py-2 text-xs"
            style={{ borderColor: "var(--m-border)", background: "var(--m-canvas)", color: "var(--m-text-secondary)" }}
          >
            Sample data — placeholders are filled with example values, not a live render.
          </div>
          <div className="grid gap-1">
            <span className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Subject
            </span>
            <p className="text-[13px]">{previewSubject.trim() === "" ? "—" : previewSubject}</p>
          </div>
          <div className="grid gap-1">
            <span className="text-[11px] uppercase tracking-wide" style={{ color: "var(--m-text-muted)" }}>
              Body
            </span>
            <PreviewBody
              html={mode === "html" ? renderDraft(body, SAMPLE_CONTEXT) : plainText}
              mode={mode}
            />
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
