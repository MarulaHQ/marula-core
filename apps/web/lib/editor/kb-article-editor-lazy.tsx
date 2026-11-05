"use client";
/**
 * Lazy boundary for the knowledge-base article editor.
 *
 * Purpose: TipTap is eleven packages and the editor only exists on `/kb` once a
 * article is open. Imported directly, the extension set and ProseMirror shipped
 * with the article ledger, so every visit to the list paid for an editor most
 * visits never open. `next/dynamic` gives the editor its own chunk and loads it
 * when the panel mounts.
 *
 * The fallback is a skeleton, not a spinner: a toolbar strip and a canvas box at
 * the same heights the real editor occupies (`.m-editor-toolbar` is a 24px tool
 * row plus 4px padding; `.m-editor-canvas` is min-height 240px), so opening an
 * article does not move the panel under the pointer. No pulse — the motion law
 * in docs/theme.md forbids it.
 *
 * Non-purpose: no editor behaviour lives here. The props are the editor's own,
 * passed straight through; `import type` keeps the TipTap module out of this
 * chunk at compile time.
 */
import dynamic from "next/dynamic";
import { Skeleton } from "@marula-core/ui";
import type { KbArticleEditorProps } from "./kb-article-editor";

function EditorSkeleton() {
  return (
    <div className="grid gap-3 p-4" role="status" aria-busy="true" aria-label="Loading article editor">
      <div
        aria-hidden="true"
        className="m-editor-toolbar"
        style={{ height: 32 }}
      >
        {[24, 24, 24, 96, 24, 24, 24, 128].map((width, index) => (
          <Skeleton
            key={index}
            shape="block"
            width={width}
            height={24}
            style={{ marginLeft: index === 3 || index === 7 ? 8 : 0 }}
          />
        ))}
      </div>
      <div
        aria-hidden="true"
        style={{
          minHeight: 240,
          background: "var(--m-surface)",
          border: "1px solid var(--m-border)",
          borderRadius: "var(--m-radius-control)",
        }}
      />
    </div>
  );
}

export const KbArticleEditor = dynamic(
  () => import("./kb-article-editor").then((m) => m.KbArticleEditor),
  { ssr: false, loading: EditorSkeleton },
);

export type { KbArticleEditorProps };
