// /projects/[id]/change-orders — the change-order and billing-release record for
// one project. The record header stays the project's; this page adds the money
// paper trail beneath it.
"use client";

import { Button, SurfaceHeader } from "@marula-core/ui";
import Link from "next/link";
import { useParams } from "next/navigation";
import { ChangeOrdersPanel } from "../../../../components/projects/change-orders-panel";

export default function ProjectChangeOrdersPage() {
  const params = useParams<{ id: string }>();
  return (
    <div className="flex flex-col gap-5 p-6">
      <SurfaceHeader
        eyebrow={
          <Link href={`/projects/${params.id}`} className="hover:underline">
            Project record
          </Link>
        }
        title="Change orders"
        description="What changed, who agreed, what it moved and what it billed."
        actions={
          <Button asChild variant="secondary">
            <Link href={`/projects/${params.id}`}>Back to the record</Link>
          </Button>
        }
      />
      <ChangeOrdersPanel projectId={params.id} />
    </div>
  );
}
