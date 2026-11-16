// /projects/templates/[id] — one template: plan, document, validation, lessons
// and versions.
"use client";

import { useParams } from "next/navigation";
import { TemplateEditor } from "../../../../components/projects/template-editor";

export default function ProjectTemplatePage() {
  const params = useParams<{ id: string }>();
  // One editor instance per template. Forking pushes a new id while the old
  // instance is still mounted, and its per-template state (the card, the
  // document, the catalogue-facts form, the open tab) belongs to the previous
  // template; the key makes the new one start clean.
  return <TemplateEditor key={params.id} />;
}
