// /projects/[id]/timeline — the project's plan as a timeline.
//
// A route of its own rather than a tab inside the record page: the chart is
// the page, the Plan tab is one link away, and both link to each other. The
// record page owns its own tab state and is edited by another lane, so this
// route repeats the header and the tab strip the Plan tab draws (see
// `ProjectTimelineShell`) instead of reaching into that file.
"use client";

import { useParams } from "next/navigation";
import { useRequireAuth } from "../../../../lib/api";
import { ProjectTimelineSurface } from "../../../../components/project-timeline-surface";

export default function ProjectTimelinePage(): React.JSX.Element | null {
  const auth = useRequireAuth();
  const params = useParams<{ id: string }>();
  if (auth.status !== "authed") return null;
  return (
    <div
      style={{
        margin: "0 auto",
        width: "100%",
        maxWidth: 1600,
        padding: 24,
        display: "grid",
        gap: 16,
      }}
    >
      <ProjectTimelineSurface projectId={params.id} />
    </div>
  );
}
