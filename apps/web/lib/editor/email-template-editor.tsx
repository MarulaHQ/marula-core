"use client";
/**
 * The rich-text body editor leaf for email templates.
 *
 * Purpose: this file is the ONLY place outside the KB editor that touches
 * TipTap (tests/components/lazy-boundaries.test.tsx enforces it). The
 * template editor page reaches it through `email-template-editor-lazy`,
 * so the eleven TipTap packages and ProseMirror load only when a template
 * editor actually mounts — the templates index never pays for them.
 *
 * The leaf is deliberately dumb about persistence: it renders StarterKit's
 * toolbar and canvas, and hands every HTML change to `onHtmlChange`. The
 * page owns the draft, the autosave envelope, and the status strip. The
 * same shape the KB editor keeps (`lib/editor/kb-article-editor.tsx`),
 * because two editors drifting into two persistence philosophies is how
 * both become wrong.
 *
 * Non-purpose: no save logic, no preview, no subject field — those live in
 * the page. The variable menu here inserts at the ProseMirror caret; the
 * subject field keeps its own shared `VariableSelect`.
 */
import { EditorContent, useEditor } from "@tiptap/react";
import type { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import {
  EditorToolbar,
  EditorToolbarGroup,
  EditorTool,
  FieldHint,
} from "@marula-core/ui";
import { VariableSelect } from "../../components/settings/email-templates/variable-select";

export interface EmailTemplateEditorProps {
  /** The template's stored HTML body, as the editor's initial document. */
  initialContent: string;
  /** The template kind's closed placeholder vocabulary. */
  vocabulary: string[];
  /** Every content change, as the serialized HTML the draft saves. */
  onHtmlChange: (html: string) => void;
}

type ToolProps = {
  editor: Editor;
  label: string;
  kind: "mark" | "toggleNode" | "toggleList" | "toggleWrap";
  name?: string;
  attrs?: Record<string, unknown>;
  children: React.ReactNode;
};

function Tool({ editor, label, kind, name, attrs, children }: ToolProps): React.JSX.Element {
  return (
    <EditorTool
      label={label}
      active={name !== undefined ? editor.isActive(name, attrs ?? {}) : false}
      onClick={() => {
        if (name === undefined) return;
        if (kind === "mark") {
          editor.chain().focus().toggleMark(name, attrs ?? {}).run();
        } else if (kind === "toggleNode") {
          editor.chain().focus().toggleNode(name, attrs as never).run();
        } else if (kind === "toggleList") {
          editor.chain().focus().toggleList(name, attrs as never).run();
        } else if (kind === "toggleWrap") {
          editor.chain().focus().toggleWrap(name, attrs as never).run();
        }
      }}
    >
      {children}
    </EditorTool>
  );
}

function LinkTool({ editor }: { editor: Editor }): React.JSX.Element {
  const active = editor.isActive("link");
  return (
    <EditorTool
      label={active ? "Remove link" : "Add link"}
      active={active}
      onClick={() => {
        if (active) {
          editor.chain().focus().unsetLink().run();
          return;
        }
        const href = window.prompt("Link URL");
        if (href === null || href.trim() === "") return;
        editor.chain().focus().setLink({ href: href.trim() }).run();
      }}
    >
      Link
    </EditorTool>
  );
}

export function EmailTemplateEditor({
  initialContent,
  vocabulary,
  onHtmlChange,
}: EmailTemplateEditorProps): React.JSX.Element {
  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        heading: { levels: [2, 3] },
        link: { openOnClick: false, autolink: true },
      }),
    ],
    content: initialContent,
    immediatelyRender: false,
    onUpdate({ editor: current }) {
      onHtmlChange(current.getHTML());
    },
  });

  const insertIntoBody = (key: string) => {
    if (!editor) return;
    editor.chain().focus().insertContent({ type: "text", text: `{{${key}}}` }).run();
  };

  return (
    <div className="grid gap-2">
      <EditorToolbar aria-label="Body formatting">
        <EditorToolbarGroup>
          {editor ? (
            <>
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
            </>
          ) : null}
        </EditorToolbarGroup>
        <EditorToolbarGroup>
          {editor ? (
            <>
              <Tool editor={editor} label="Heading" kind="toggleNode" name="heading" attrs={{ level: 2 }}>
                H2
              </Tool>
              <Tool editor={editor} label="Subheading" kind="toggleNode" name="heading" attrs={{ level: 3 }}>
                H3
              </Tool>
              <Tool editor={editor} label="Bullet list" kind="toggleList" name="bulletList">
                •
              </Tool>
              <Tool editor={editor} label="Numbered list" kind="toggleList" name="orderedList">
                1.
              </Tool>
              <Tool editor={editor} label="Blockquote" kind="toggleWrap" name="blockquote">
                ❝
              </Tool>
            </>
          ) : null}
        </EditorToolbarGroup>
        <EditorToolbarGroup>
          {editor ? <LinkTool editor={editor} /> : null}
        </EditorToolbarGroup>
        <EditorToolbarGroup>
          <VariableSelect
            vocabulary={vocabulary}
            onInsert={insertIntoBody}
            ariaLabel="Insert variable into body"
          />
        </EditorToolbarGroup>
      </EditorToolbar>
      <EditorContent editor={editor} className="m-editor-canvas" aria-label="Template body editor" />
      <FieldHint>
        Undo and redo with the editor's own shortcuts (Ctrl/Cmd+Z and
        Ctrl/Cmd+Shift+Z). Placeholders look like {"{{client.name}}"} — insert
        them from the variable menu so the spelling is always right.
      </FieldHint>
    </div>
  );
}
