"use client";
/**
 * /kb article editor (ADR 0007, Wave 16 lane 2) — Tiptap headless shell
 * composed from `meridianExtensionSet`, riding Workbench/Tangerine
 * primitives from @meridian/ui.
 *
 * Autosave rides the instant-first envelope (IF1–IF3): each debounced
 * content transaction serializes to the canonical envelope and PATCHes
 * `/api/v1/kb/articles/{id}` under the idempotency key + If-Match
 * version — debounce bursts coalesce into one write, replays stay
 * byte-identical, and undo history stays on the SERVER autosave read
 * model (mutation.before/after — IF7/IF8); no sidecar stack here.
 * Native editor undo (Ctrl/Cmd+Z) stays ProseMirror-local for
 * uncommitted transactions; committed ones reconcile through the
 * envelope.
 *
 * Non-purpose: asset/taxonomy picker dialogs ride the close-out lane's
 * surfaces; secrets stay out; read-only bundles never import this file
 * (single-article reads render server-sanitized `content_html`).
 */
import { EditorContent, useEditor } from "@tiptap/react";
import type { Editor } from "@tiptap/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  EditorToolbar,
  EditorToolbarGroup,
  EditorTool,
  Spinner,
} from "@meridian/ui";
import type { components } from "@meridian/sdk";
import {
  rememberVersion,
  runEnvelopeMutation,
} from "../envelope-write";
import { meridianExtensionSet } from "./extensions";

export interface KbArticleEditorProps {
  /** Full article detail (flattened schema carries content_jsonb). */
  article: components["schemas"]["KbArticle"];
  onSaved(): void;
  /** Typed rejection surface (conflict/validation) — never silent. */
  onRejection(friendly: string | null, reason: string | null): void;
}

const AUTOSAVE_MS = 700;

/** The canonical storage envelope: `{"schema_ver": 1, "doc": <tiptap>}`. */
function envelopeOf(doc: unknown): { schema_ver: 1; doc: unknown } {
  return { schema_ver: 1, doc };
}

/** The 0077 wrap shape for legacy rows without an envelope. */
function fallbackDoc(body: string): unknown {
  if (body.trim() === "") {
    return { type: "doc", content: [{ type: "paragraph" }] };
  }
  return {
    type: "doc",
    content: body.split("\n").map((line) => ({
      type: "paragraph",
      ...(line === "" ? {} : { content: [{ type: "text", text: line }] }),
    })),
  };
}

export function KbArticleEditor(props: KbArticleEditorProps) {
  const initialDoc = useMemo(() => {
    const envelope = props.article.content_jsonb as
      | { schema_ver?: number; doc?: unknown }
      | null
      | undefined;
    if (envelope?.doc !== undefined) {
      return envelope.doc as never;
    }
    return fallbackDoc(props.article.body ?? "") as never;
  }, [props.article.body, props.article.content_jsonb]);

  const [status, setStatus] = useState<"idle" | "saving" | "saved" | "error">(
    "idle",
  );
  const [error, setError] = useState<string | null>(null);
  const versionRef = useRef<number | null>(null);
  const inFlightRef = useRef(false);
  const dirtyRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestJsonRef = useRef<unknown>(initialDoc);

  const scheduleFlush = useCallback(() => {
    if (inFlightRef.current) {
      dirtyRef.current = true; // the flush-after-settle path picks it up
      return;
    }
    if (timerRef.current !== null) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      void flush();
    }, AUTOSAVE_MS);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const flush = useCallback(async () => {
    if (inFlightRef.current || !dirtyRef.current) return;
    inFlightRef.current = true;
    dirtyRef.current = false;
    setStatus("saving");
    const entityId = props.article.id;
    const envelope = envelopeOf(latestJsonRef.current);
    const result = await runEnvelopeMutation<
      components["schemas"]["UpdateArticleRequest"],
      components["schemas"]["ArticleMutationResponse"]
    >({
      describe: "KB article autosave",
      entityId,
      buildBody: (base) => ({
        idempotency_key: base.idempotency_key,
        ...(versionRef.current !== null ? { version: versionRef.current } : {}),
        content_jsonb: envelope,
      }) as unknown as components["schemas"]["UpdateArticleRequest"],
      optimistic: () => {},
      revert: () => {},
      request: (client, body) =>
        client.PATCH("/api/v1/kb/articles/{id}", {
          params: { path: { id: entityId } },
          body,
        }),
      versions: (response) => {
        versionRef.current = response.mutation.version;
        rememberVersion(entityId, response.mutation.version);
        return [[entityId, response.mutation.version]];
      },
    });
    inFlightRef.current = false;
    if (!result.ok) {
      setStatus("idle");
      setError(result.friendly ?? result.message ?? "The save was rejected.");
      props.onRejection(
        result.friendly ?? result.message ?? null,
        result.reason ?? null,
      );
      return;
    }
    props.onSaved();
    setStatus("saved");
    setError(null);
    // Edits during the wire already set dirtyRef — flush the newest
    // transaction now (last-write-wins, byte-identical envelope).
    if (dirtyRef.current) {
      dirtyRef.current = false;
      scheduleFlush();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.article.id]);

  const editor = useEditor({
    extensions: meridianExtensionSet,
    content: initialDoc,
    immediatelyRender: false,
    onUpdate({ editor: current }) {
      latestJsonRef.current = current.getJSON();
      dirtyRef.current = true;
      setStatus("idle");
      scheduleFlush();
    },
  });

  useEffect(() => {
    versionRef.current = null;
    rememberVersion(props.article.id, null);
    return () => {
      if (timerRef.current !== null) clearTimeout(timerRef.current);
    };
  }, [props.article.id]);

  if (editor === null) return <Spinner />;

  return (
    <div className="grid gap-3 p-4">
      <SaveStrip status={status} error={error} />
      <EditorToolbar aria-label="Article formatting">
        <EditorToolbarGroup>
          <Tool editor={editor} label="Bold" kind="mark" name="bold">
            <strong>B</strong>
          </Tool>
          <Tool editor={editor} label="Italic" kind="mark" name="italic">
            <em>I</em>
          </Tool>
          <Tool editor={editor} label="Underline" kind="mark" name="underline">
            <u>U</u>
          </Tool>
          <Tool editor={editor} label="Strikethrough" kind="mark" name="strike">
            <s>S</s>
          </Tool>
          <Tool editor={editor} label="Inline code" kind="mark" name="code">
            {"<>"}
          </Tool>
        </EditorToolbarGroup>
        <EditorToolbarGroup>
          <Tool
            editor={editor}
            label="Heading"
            kind="toggleNode"
            name="heading"
            attrs={{ level: 2 }}
          >
            H2
          </Tool>
          <Tool
            editor={editor}
            label="Subheading"
            kind="toggleNode"
            name="heading"
            attrs={{ level: 3 }}
          >
            H3
          </Tool>
        </EditorToolbarGroup>
        <EditorToolbarGroup>
          <Tool editor={editor} label="Bullet list" kind="toggleList" name="bulletList">
            •
          </Tool>
          <Tool editor={editor} label="Numbered list" kind="toggleList" name="orderedList">
            1.
          </Tool>
          <Tool editor={editor} label="Task list" kind="toggleWrap" name="taskList">
            ☑
          </Tool>
          <Tool editor={editor} label="Blockquote" kind="toggleWrap" name="blockquote">
            ❝
          </Tool>
          <Tool editor={editor} label="Code block" kind="setNode" name="codeBlock">
            {"{ }"}
          </Tool>
          <Tool
            editor={editor}
            label="Horizontal rule"
            kind="command"
            name="setHorizontalRule"
          >
            —
          </Tool>
          <Tool editor={editor} label="YouTube embed" kind="youtube">
            ▶
          </Tool>
        </EditorToolbarGroup>
      </EditorToolbar>
      <EditorContent editor={editor} className="m-editor-canvas" />
    </div>
  );
}

function SaveStrip({
  status,
  error,
}: {
  status: "idle" | "saving" | "saved" | "error";
  error: string | null;
}) {
  if (error !== null) {
    return (
      <p
        role="alert"
        className="text-[12px]"
        style={{ color: "var(--m-status-danger-fg)" }}
      >
        {error}
      </p>
    );
  }
  if (status === "saving") {
    return (
      <p
        className="flex items-center gap-2 text-[12px]"
        style={{ color: "var(--m-text-secondary)" }}
        aria-live="polite"
      >
        <Spinner /> Saving…
      </p>
    );
  }
  if (status === "saved") {
    return (
      <p
        className="text-[12px]"
        style={{ color: "var(--m-status-success-fg)" }}
        aria-live="polite"
      >
        Saved
      </p>
    );
  }
  return null;
}

type ToolProps = {
  editor: Editor;
  label: string;
  kind:
    | "mark"
    | "toggleNode"
    | "toggleList"
    | "toggleWrap"
    | "setNode"
    | "command"
    | "youtube";
  name?: string;
  attrs?: Record<string, unknown>;
  children: React.ReactNode;
};

function Tool({ editor, label, kind, name, attrs, children }: ToolProps) {
  const active =
    name !== undefined && kind !== "command" && kind !== "youtube"
      ? editor.isActive(name, attrs ?? {})
      : false;
  return (
    <EditorTool
      label={label}
      active={active}
      onClick={() => {
        // All editor state flows through Tiptap commands — the picker
        // dialog (setYouTubeVideo variants) rides the close-out lane;
        // the toolbar marks this tool disabled-until-primitive.
        if (kind === "youtube") return;
        if (kind === "mark" && name) {
          editor.chain().focus().toggleMark(name, attrs ?? {}).run();
        } else if (kind === "toggleNode" && name) {
          editor.chain().focus().toggleNode(name, attrs as never).run();
        } else if (kind === "toggleList" && name) {
          editor.chain().focus().toggleList(name, attrs as never).run();
        } else if (kind === "toggleWrap" && name) {
          editor.chain().focus().toggleWrap(name, attrs as never).run();
        } else if (kind === "setNode" && name) {
          editor.chain().focus().setNode(name, attrs as never).run();
        } else if (kind === "command" && name) {
          const chain = editor.chain().focus() as unknown as Record<
            string,
            (() => { run(): void }) | undefined
          >;
          chain[name]?.().run();
        }
      }}
    >
      {children}
    </EditorTool>
  );
}
