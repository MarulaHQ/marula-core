// /portal/projects — the client's projects, in plain words (S16.1/S16.2
// entry points).
//
// Nothing internal appears here: no hours, no budget, no technician names. A
// project that needs her answer is the one highlighted row.
"use client";

import Link from "next/link";
import { useEffect, useState } from "react";
import { useRequirePortalAuth } from "../../../lib/portal-api";
import { longDate } from "../../../lib/portal-copy";
import { useTimeFormat } from "../../../lib/use-time-format";
import { PortalCard, PortalEmpty, PortalRow } from "../../../components/portal/portal-ui";
import { listPortalProjects, type PortalProjectSummary } from "./portal-projects-api";

export default function PortalProjectsPage() {
  useRequirePortalAuth();
  const { timeZone } = useTimeFormat();
  const [projects, setProjects] = useState<PortalProjectSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listPortalProjects()
      .then((body) => setProjects(body.projects))
      .catch((e: Error) => setError(e.message));
  }, []);

  return (
    <div className="portal-page">
      <div className="portal-heading-row">
        <div>
          <h1 className="portal-title">Your projects</h1>
          <p className="portal-lead">
            What we are delivering, where each one has got to, and the date that
            matters.
          </p>
        </div>
      </div>

      {error !== null ? (
        <PortalCard title="We could not load your projects">
          <p className="portal-hint" role="alert">
            {error}
          </p>
        </PortalCard>
      ) : null}

      {projects === null && error === null ? (
        <p className="portal-hint">Loading your projects…</p>
      ) : null}

      {projects !== null && projects.length === 0 ? (
        <PortalEmpty title="No projects yet">
          When work starts, it appears here with what it means for you and when you
          will hear from us next.
        </PortalEmpty>
      ) : null}

      {projects !== null && projects.length > 0 ? (
        <PortalCard title="In progress and completed">
          {projects.map((project) => (
            <PortalRow
              key={project.id}
              title={project.name}
              meta={`${project.status_text}${
                project.due_date === null ? "" : ` · ${longDate(project.due_date, timeZone)}`
              }`}
              highlight={project.waiting_for_you}
              trailing={
                <span className="portal-hint">
                  {project.waiting_for_you ? (
                    <b>Waiting for your answer</b>
                  ) : project.latest_update_at === null ? (
                    "No update yet"
                  ) : (
                    `Updated ${longDate(project.latest_update_at.slice(0, 10), timeZone)}`
                  )}
                </span>
              }
              onClick={undefined}
              to={`/portal/projects/${project.id}`}
            />
          ))}
          <p className="portal-hint">
            <Link href="/portal">Back to your home</Link>
          </p>
        </PortalCard>
      ) : null}
    </div>
  );
}
