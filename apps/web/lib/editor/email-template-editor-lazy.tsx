"use client";
/**
 * Lazy boundary for the email-template body editor.
 *
 * Same contract as `kb-article-editor-lazy`: TipTap is eleven packages and
 * only a template editor open pays for them. `next/dynamic` gives the leaf
 * its own chunk and loads it when the editor card mounts. The fallback is
 * a skeleton at the same heights the real editor occupies (`.m-editor-toolbar`
 * tool row plus `.m-editor-canvas` at min-height 240px), so mounting does
 * not move the page under the pointer. No pulse — the motion law in
 * docs/theme.md forbids it.
 *
 * Non-purpose: no editor behaviour lives here. The props are the leaf's
 * own, passed straight through; `import type` keeps TipTap out of this
 * chunk at compile time.
 */
import dynamic from "next/dynamic";
import { Skeleton } from "@marula-core/ui";
import type { EmailTemplateEditorProps } from "./email-template-editor";

function EditorSkeleton() {
  return (
    <div className="grid gap-3 p-4" role="status" aria-busy="true" aria-label="Loading template body editor">
      <div aria-hidden="true" className="m-editor-toolbar" style={{ height: 32 }}>
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
        className="m-editor-canvas"
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

export const EmailTemplateEditorLazy = dynamic<EmailTemplateEditorProps>(
  () => import("./email-template-editor").then((m) => m.EmailTemplateEditor),
  { ssr: false, loading: EditorSkeleton },
);

export type { EmailTemplateEditorProps };
