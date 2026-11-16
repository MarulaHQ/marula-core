// /projects/templates — the gallery route. All state lives in the component so
// the page stays a one-line shell that the templates surface can be tested
// without a router.
"use client";

import { TemplateGallery } from "../../../components/projects/template-gallery";

export default function ProjectTemplatesPage() {
  return <TemplateGallery />;
}
