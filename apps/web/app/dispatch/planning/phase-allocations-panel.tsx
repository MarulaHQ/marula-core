"use client";

// Phase allocations — who is committed to which project phase
// (Wave 34 claims close-out, claim 1).
//
// Purpose: the editable half of the resource-planning pair. The forecast
// beside it reports these numbers; this panel is where a planner states
// them.
//
// Re-stating an allocation REPLACES it rather than adding a second. Two rows
// for one (phase, technician) pair would sum in the forecast and report
// someone as committed to 110% of their time by construction, so the
// upsert semantics here are load-bearing rather than a convenience.
//
// Non-purpose: no project CRUD. A phase has to exist before it can be
// allocated, and the project page is where phases are made.

import {
  Badge,
  Button,
  Card,
  CardHeader,
  EmptyState,
  Input,
  Spinner,
  Table,
  Td,
  Th,
  Tr,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../../../lib/api";

type AllocationList = components["schemas"]["PhaseAllocationListView"];
type Allocation = components["schemas"]["PhaseAllocationView"];
type ProjectGantt = components["schemas"]["ProjectGanttView"];
type Phase = components["schemas"]["PhaseGanttView"];

export function PhaseAllocationsPanel({
  onError,
}: {
  onError?: (message: string) => void;
}) {
  const [list, setList] = useState<AllocationList | null>(null);
  // The FLATTENED phases of whichever project is picked, dated and undated
  // alike. A phase with no dates is still allocatable: "Alice owns the
  // migration" is a real state and the forecast counts it for the whole window.
  const [phases, setPhases] = useState<Phase[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);

  // The form is driven by the projects the caller picks, so the phase list is
  // a dependent fetch rather than a flat picker over every phase in the org.
  const [projectId, setProjectId] = useState("");
  const [phaseId, setPhaseId] = useState("");
  const [technicianId, setTechnicianId] = useState("");
  const [share, setShare] = useState("50");
  const [start, setStart] = useState("");
  const [end, setEnd] = useState("");

  const [projectOptions, setProjectOptions] = useState<
    { id: string; name: string }[]
  >([]);
  const [technicians, setTechnicians] = useState<
    { id: string; display_name: string }[]
  >([]);

  const load = useCallback(async () => {
    try {
      const [allocations, projs, users] = await Promise.all([
        api.api.GET("/api/v1/dispatch/planning/allocations"),
        api.api.GET("/api/v1/projects", { params: { query: { limit: 200 } } }),
        api.api.GET("/api/v1/org/users", { params: { query: {} } }),
      ]);
      if (allocations.data) setList(allocations.data as AllocationList);
      if (Array.isArray(projs.data)) {
        setProjectOptions(
          (projs.data as { id: string; name: string }[]).map((p) => ({
            id: p.id,
            name: p.name,
          })),
        );
      }
      if (users.data !== undefined && Array.isArray(users.data.users)) {
        // `/org/users` returns a PAGE, not a bare array, and the real shape
        // is already in the SDK types — so no cast is needed and a rename
        // upstream becomes a type error here rather than an undefined read.
        setTechnicians(
          users.data.users.map((u) => ({
            id: u.id,
            // An inactive technician is not a candidate for new work, so the
            // picker shows only active ones rather than offering someone the
            // board would not route to.
            display_name: u.display_name,
          })),
        );
      }
    } catch (e) {
      onError?.(e instanceof Error ? e.message : "could not load allocations");
    } finally {
      setLoading(false);
    }
  }, [onError]);

  useEffect(() => {
    void load();
  }, [load]);

  // Load the chosen project's phases, dated and undated alike: a phase with
  // no dates is still allocatable, because "Alice owns the migration" is a
  // real state and the forecast counts it for the whole window.
  useEffect(() => {
    if (!projectId) {
      setPhases([]);
      return;
    }
    (async () => {
      try {
        const res = await api.api.GET("/api/v1/projects/{id}/gantt", {
          params: { path: { id: projectId } },
        });
        if (res.data) {
          const g = res.data as ProjectGantt;
          setPhases([...g.dated, ...g.undated]);
        }
      } catch (e) {
        onError?.(e instanceof Error ? e.message : "could not load phases");
      }
    })();
  }, [projectId, onError]);

  const phaseOptions = useMemo(
    () =>
      phases.map((p) => ({
        value: p.id,
        label: p.start_date ? p.name : `${p.name} (undated)`,
      })),
    [phases],
  );

  // Native selects rather than the Radix kit: this is a plain "pick one of
  // N" form, and the app's own settings pages use native selects for
  // exactly this shape. The Radix Select is for searchable comboboxes.
  const fieldStyle = {
    display: "grid",
    gap: "4px",
    fontSize: "12px",
    alignContent: "start",
  } as const;

  // The native <select> and the kit <Input> are different elements with
  // different default box metrics. Matching the visible box is what makes
  // the row read as one form instead of five unrelated controls.
  const controlStyle = {
    width: "100%",
    minHeight: "32px",
    padding: "0 8px",
    fontSize: "13px",
    borderRadius: "var(--m-radius-sm)",
    border: "1px solid var(--m-border)",
    background: "var(--m-surface)",
    color: "var(--m-text)",
  } as const;

  const submit = useCallback(async () => {
    if (!phaseId || !technicianId) return;
    const shareNum = Number(share);
    if (!Number.isInteger(shareNum) || shareNum < 1 || shareNum > 100) {
      onError?.("the share must be a whole number from 1 to 100");
      return;
    }
    // Both dates or neither. A phase with a start and no end has no length,
    // and the server rejects it — the form says so before the round trip.
    if (start !== "" !== (end !== "")) {
      onError?.("give both dates or neither — a phase window needs a length");
      return;
    }
    setSaving(true);
    try {
      await api.api.PUT("/api/v1/dispatch/planning/allocations", {
        body: {
          phase_id: phaseId,
          user_id: technicianId,
          share_percent: shareNum,
          start_date: start === "" ? null : start,
          end_date: end === "" ? null : end,
          note: null,
        },
      });
      await load();
      setShare("50");
    } catch (e) {
      onError?.(e instanceof Error ? e.message : "could not save the allocation");
    } finally {
      setSaving(false);
    }
  }, [phaseId, technicianId, share, start, end, load, onError]);

  const drop = useCallback(
    async (a: Allocation) => {
      try {
        await api.api.DELETE(
          "/api/v1/dispatch/planning/allocations/{allocation_id}",
          { params: { path: { allocation_id: a.id } } },
        );
        await load();
      } catch (e) {
        onError?.(e instanceof Error ? e.message : "could not release it");
      }
    },
    [load, onError],
  );

  if (loading) return <Spinner label="Loading allocations" />;

  const rows = list?.allocations ?? [];

  return (
    <Card>
      <CardHeader>
        <h2 className="p-4 pb-0 text-sm font-semibold">
          Phase allocations
          <span className="ml-3 font-normal" style={{ color: "var(--m-text-secondary)" }}>
            Stated commitments, in addition to booked appointments
          </span>
        </h2>
      </CardHeader>
      {rows.length === 0 ? (
        <EmptyState message="No project commitments yet. State who is working on which phase, and the forecast beside this will include it." />
      ) : (
        <Table>
          <thead>
            <tr>
              <Th>Technician</Th>
              <Th>Project</Th>
              <Th>Phase</Th>
              <Th>Share</Th>
              <Th>Window</Th>
              <Th />
            </tr>
          </thead>
          <tbody>
            {rows.map((a) => (
              <Tr key={a.id}>
                <Td>{a.user_display_name}</Td>
                <Td>{a.project_name}</Td>
                <Td>
                  {a.phase_name}
                  {a.phase_status === "completed" ? (
                    <Badge tone="neutral" style={{ marginLeft: "6px" }}>
                      done
                    </Badge>
                  ) : null}
                </Td>
                <Td style={{ fontVariantNumeric: "tabular-nums" }}>
                  {a.share_percent}%
                </Td>
                <Td>
                  {a.start_date && a.end_date ? (
                    `${a.start_date} → ${a.end_date}`
                  ) : (
                    <span style={{ color: "var(--m-text-secondary)" }}>
                      open-ended
                    </span>
                  )}
                </Td>
                <Td>
                  <Button
                    size="sm"
                    variant="ghost"
                    aria-label={`Release ${a.user_display_name} from ${a.phase_name}`}
                    onClick={() => drop(a)}
                  >
                    Release
                  </Button>
                </Td>
              </Tr>
            ))}
          </tbody>
        </Table>
      )}

      <div
        style={{
          marginTop: "12px",
          paddingTop: "12px",
          borderTop: "1px solid var(--m-border)",
          display: "grid",
          gap: "8px",
          gridTemplateColumns: "repeat(auto-fit, minmax(150px, 1fr))",
          // `stretch`, not `end`: the native selects and the date/number
          // inputs have different intrinsic heights, and bottom-aligning
          // them dropped the select labels BELOW their own controls while
          // the input labels stayed above. Caught by the vision pass.
          alignItems: "stretch",
        }}
      >
        <label style={fieldStyle}>
          Project
          <select
            style={controlStyle}
            value={projectId}
            onChange={(e) => {
              setProjectId(e.target.value);
              setPhaseId("");
            }}
          >
            <option value="">Choose a project…</option>
            {projectOptions.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>

        <label style={fieldStyle}>
          Phase
          <select
            style={controlStyle}
            value={phaseId}
            disabled={!projectId || phaseOptions.length === 0}
            onChange={(e) => setPhaseId(e.target.value)}
          >
            <option value="">
              {projectId ? "Choose a phase…" : "Pick a project first"}
            </option>
            {phaseOptions.map((p) => (
              <option key={p.value} value={p.value}>
                {p.label}
              </option>
            ))}
          </select>
        </label>

        <label style={fieldStyle}>
          Technician
          <select
            style={controlStyle}
            value={technicianId}
            onChange={(e) => setTechnicianId(e.target.value)}
          >
            <option value="">Choose a technician…</option>
            {technicians.map((t) => (
              <option key={t.id} value={t.id}>
                {t.display_name}
              </option>
            ))}
          </select>
        </label>

        <label style={fieldStyle}>
          Share %
          <Input
            type="number"
            min={1}
            max={100}
            value={share}
            onChange={(e) => setShare(e.target.value)}
          />
        </label>

        <label style={fieldStyle}>
          From
          <Input
            type="date"
            value={start}
            onChange={(e) => setStart(e.target.value)}
          />
        </label>

        <label style={fieldStyle}>
          To
          <Input
            type="date"
            value={end}
            onChange={(e) => setEnd(e.target.value)}
          />
        </label>

        <div style={{ display: "flex", alignItems: "flex-end" }}>
          <Button
            onClick={submit}
            disabled={saving || !phaseId || !technicianId}
            style={{ justifySelf: "start" }}
          >
            {saving ? "Saving…" : "State allocation"}
          </Button>
        </div>
      </div>

      <p
        style={{
          color: "var(--m-text-secondary)",
          fontSize: "12px",
          marginTop: "8px",
        }}
      >
        Re-stating a technician on a phase replaces the previous share rather
        than adding to it. Leave the dates blank for an open-ended commitment,
        which counts against the whole window.
      </p>
    </Card>
  );
}
