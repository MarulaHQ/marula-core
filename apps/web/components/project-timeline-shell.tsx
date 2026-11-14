"use client";
/**
 * ProjectTimelineShell — the identity header and tab strip the Timeline route
 * draws above its chart.
 *
 * Purpose: the Plan tab and the Timeline tab must read as the same record, so
 * this repeats the Plan tab's own header composition — breadcrumb, title and
 * its pills, the meta line, the phase roll-up, and the tab strip with a
 * 2px underline on the current tab — rather than inventing a second header.
 * The strip is anchors, not buttons, because on this route the tabs are
 * routes: Plan goes back to `/projects/{id}`, Timeline is here.
 *
 * Non-purpose: it does not fetch and holds no tab state. The Plan route keeps
 * its tabs in component state and is owned by another lane, so only the two
 * URL-addressable tabs are offered here; the record's Tasks and money tabs are
 * one click away on Plan, named in the strip's trailing link.
 */
import Link from "next/link";
import { Badge } from "@marula-core/ui";
import { Icon } from "@marula-core/ui/kit";
import { projectRisk, projectStatusTone } from "../lib/projects-metrics";
import type { TimelinePhase, TimelineProject } from "../lib/projects-timeline-types";

export const PROJECT_STATUS_LABEL: Record<string, string> = {
  planning: "Planning",
  active: "Active",
  on_hold: "On hold",
  completed: "Completed",
  cancelled: "Cancelled",
};

export function ProjectTimelineShell({
  project,
  phases,
  taskCount,
  baselineLabel,
  children,
}: {
  project: TimelineProject;
  phases: TimelinePhase[];
  taskCount: number;
  baselineLabel: string | null;
  children: React.ReactNode;
}): React.JSX.Element {
  const risk = projectRisk(project);
  const phasesDone = phases.filter((phase) => phase.status === "completed").length;
  const nextMilestone = phases
    .filter((phase) => phase.is_milestone && phase.status !== "completed")
    .sort((a, b) => (a.start_date ?? "9999").localeCompare(b.start_date ?? "9999"))[0];

  return (
    <div style={{ display: "grid", gap: 16 }}>
      <div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            color: "var(--m-text-secondary)",
            fontSize: 13,
          }}
        >
          <Link
            href="/projects"
            style={{ color: "inherit", textDecoration: "underline", textUnderlineOffset: 2 }}
          >
            Projects
          </Link>
          <Icon name="chev" size={14} />
          <span>{project.name}</span>
        </div>

        <div style={{ marginTop: 8, display: "flex", alignItems: "center", flexWrap: "wrap", gap: 10 }}>
          <h1
            style={{
              margin: 0,
              fontSize: 26,
              lineHeight: 1.15,
              letterSpacing: "-0.01em",
              fontWeight: 600,
            }}
          >
            {project.name}
          </h1>
          <Badge tone={projectStatusTone(project.status)}>
            {PROJECT_STATUS_LABEL[project.status] ?? project.status.replaceAll("_", " ")}
          </Badge>
          {risk !== null ? (
            <Badge tone={risk.tone === "danger" ? "danger" : "warning"}>{risk.label}</Badge>
          ) : null}
        </div>

        <p style={{ margin: "4px 0 0", fontSize: 13, color: "var(--m-text-secondary)" }}>
          {project.start_date !== null ? `Starts ${project.start_date}` : "No start date"}
          {project.due_date !== null ? ` · due ${project.due_date}` : ""}
          {` · ${phasesDone}/${phases.length} phases · ${taskCount} ${taskCount === 1 ? "task" : "tasks"}`}
        </p>
      </div>

      <TimelineTabs projectId={project.id} />

      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          alignItems: "center",
          gap: 18,
          padding: "10px 0",
          borderBottom: "1px solid var(--m-border)",
          fontSize: 13,
        }}
      >
        <Commitment label="Started" value={project.start_date ?? "Not dated"} />
        <Separator />
        <Commitment label="Promised go-live" value={project.due_date ?? "Not dated"} />
        {nextMilestone !== undefined ? (
          <>
            <Separator />
            <span style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <span aria-hidden="true" style={{ color: "var(--m-text)" }}>
                ◆
              </span>
              <span style={{ color: "var(--m-text-muted)", fontSize: 12 }}>Next milestone</span>
              <span style={{ fontWeight: 500 }}>{nextMilestone.name}</span>
              <span
                style={{
                  fontFamily: "var(--m-font-mono)",
                  fontSize: 12,
                  fontVariantNumeric: "tabular-nums",
                  color: "var(--m-text-secondary)",
                }}
              >
                {nextMilestone.start_date ?? "Not dated"}
              </span>
            </span>
          </>
        ) : null}
        <span style={{ marginLeft: "auto" }}>
          <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>
            {baselineLabel === null
              ? "No baseline captured"
              : `Baseline captured ${baselineLabel}`}
          </span>
        </span>
      </div>

      {children}
    </div>
  );
}

/** The record's tab strip. The Plan tab is a real link back to the record;
 * Timeline is the current page. Only tabs with a route are rendered. */
export function TimelineTabs({ projectId }: { projectId: string }): React.JSX.Element {
  return (
    <nav
      aria-label="Record sections"
      data-testid="record-tabs"
      style={{
        display: "flex",
        flexWrap: "wrap",
        gap: "0 16px",
        borderBottom: "1px solid var(--m-border)",
      }}
    >
      <TabLink href={`/projects/${projectId}`} label="Plan" active={false} />
      <TabLink href={`/projects/${projectId}/timeline`} label="Timeline" active />
      <span
        style={{
          marginLeft: "auto",
          alignSelf: "center",
          paddingBottom: 8,
          fontSize: 11,
          color: "var(--m-text-muted)",
        }}
      >
        <Link
          href={`/projects/${projectId}`}
          style={{ color: "inherit", textDecoration: "underline", textUnderlineOffset: 2 }}
        >
          Tasks and money
        </Link>{" "}
        live on the project record
      </span>
    </nav>
  );
}

function TabLink({
  href,
  label,
  active,
}: {
  href: string;
  label: string;
  active: boolean;
}): React.JSX.Element {
  return (
    <Link
      href={href}
      aria-current={active ? "page" : undefined}
      data-testid={`record-tab-${label.toLowerCase()}`}
      style={{
        padding: "4px 2px 8px",
        borderBottom: `2px solid ${active ? "var(--m-text)" : "transparent"}`,
        color: active ? "var(--m-text)" : "var(--m-text-secondary)",
        fontWeight: active ? 600 : 500,
        fontSize: 13,
        textDecoration: "none",
        whiteSpace: "nowrap",
      }}
    >
      {label}
    </Link>
  );
}

function Commitment({ label, value }: { label: string; value: string }): React.JSX.Element {
  return (
    <span style={{ display: "flex", alignItems: "baseline", gap: 8 }}>
      <span style={{ fontSize: 12, color: "var(--m-text-muted)" }}>{label}</span>
      <span
        style={{
          fontFamily: "var(--m-font-mono)",
          fontSize: 12,
          fontVariantNumeric: "tabular-nums",
        }}
      >
        {value}
      </span>
    </span>
  );
}

function Separator(): React.JSX.Element {
  return (
    <span
      aria-hidden="true"
      style={{ width: 1, height: 16, background: "var(--m-border)" }}
    />
  );
}
