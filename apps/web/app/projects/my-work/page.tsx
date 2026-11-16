// /projects/my-work — the standalone project-shaped day.
"use client";

import { SurfaceHeader } from "@marula-core/ui";
import { MyWorkList } from "../../../components/projects/my-work";

export default function MyProjectWorkPage() {
  return (
    <div className="flex flex-col gap-5 p-6">
      <SurfaceHeader
        title="My project work"
        description="What needs you across every project, with waiting as its own state."
      />
      <MyWorkList />
    </div>
  );
}
