"use client";
// /kb/racks/[id] — one rack, drawn: the elevation, the devices and the power.
//
// Purpose: the pack's K05. A rack page is shaped for a rack, not for a server:
// the front and rear elevations are the page, the device table is the same
// information in rows, and the right column carries the power load and the
// records this rack is linked to.
//
// Non-purpose: no drag-and-drop. Placement is a small form with a unit number,
// because a drag on an elevation is not a keyboard-reachable operation.

import Link from "next/link";
import { useParams } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { useRequireAuth } from "../../../../lib/api";
import { SurfaceHeader } from "@marula-core/ui";
import {
  ErrorNote,
  HEAD_CELL_CLASS,
  PageFrame,
  SkeletonRows,
  StatusPill,
} from "../../../../components/knowledge/ledger";
import { RackElevation } from "../../../../components/knowledge/rack-elevation";
import {
  knowledgeApi,
  type KnowledgeRequestError,
  type RackDetail,
  type RackDevice,
} from "../../../../components/knowledge/knowledge-api";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  SummaryCard,
} from "../../../../components/records/record-frame";

const TYPE_OPTIONS: Array<RackDevice["device_type"]> = [
  "network",
  "compute",
  "storage",
  "power",
  "patch",
  "other",
];

const DEVICE_TYPES = TYPE_OPTIONS;

export default function RackPage() {
  useRequireAuth();
  const params = useParams<{ id: string }>();
  const rackId = params.id;
  const [rack, setRack] = useState<RackDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [face, setFace] = useState<"front" | "rear">("front");
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(() => {
    if (rackId === undefined) return;
    setError(null);
    knowledgeApi
      .get<RackDetail>(`/api/v1/knowledge/racks/${rackId}`)
      .then(setRack)
      .catch((e: KnowledgeRequestError) => setError(e.message));
  }, [rackId]);

  useEffect(load, [load]);

  async function addDevice(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    setFormError(null);
    setSaving(true);
    try {
      await knowledgeApi.post(`/api/v1/knowledge/racks/${rackId}/devices`, {
        name: String(data.get("name") ?? ""),
        device_type: String(data.get("device_type") ?? "compute"),
        face,
        unit_start: Number(data.get("unit_start") ?? 1),
        unit_height: Number(data.get("unit_height") ?? 1),
        model: String(data.get("model") ?? "") || null,
        status: "online",
      });
      form.reset();
      load();
    } catch (e) {
      setFormError((e as KnowledgeRequestError).message);
    } finally {
      setSaving(false);
    }
  }

  if (error !== null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <ErrorNote title="The rack did not load.">{error}</ErrorNote>
        </div>
      </PageFrame>
    );
  }

  if (rack === null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <SkeletonRows rows={7} sentence="Loading the rack" columns={3} />
        </div>
      </PageFrame>
    );
  }

  const loadPct =
    rack.power_capacity_kw !== null && rack.power_capacity_kw > 0 && rack.power_used_kw !== null
      ? Math.round((rack.power_used_kw / rack.power_capacity_kw) * 100)
      : null;

  return (
    <PageFrame>
      <div className="flex flex-col gap-3 pb-4 pt-5">
        <nav aria-label="Breadcrumb" className="flex items-center gap-1.5 text-[13px]">
          <Link href="/kb" className="underline underline-offset-2" style={{ color: "var(--m-text-secondary)" }}>
            Knowledge
          </Link>
          <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
            /
          </span>
          <Link
            href={`/kb/racks?client_id=${rack.client_id}`}
            className="underline underline-offset-2"
            style={{ color: "var(--m-text-secondary)" }}
          >
            Racks
          </Link>
          <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
            /
          </span>
          <span className="font-semibold">{rack.name}</span>
        </nav>

        <RecordFrame
          context={
            <>
              <RecordSection label="Rack">
                <p className="text-[18px] font-semibold leading-tight">{rack.name}</p>
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {rack.location_note ?? "No location recorded"}
                </p>
              </RecordSection>
              <RecordSection label="Details">
                <RecordRow label="Size">{rack.units}U</RecordRow>
                <RecordRow label="Dimensions">
                  {rack.width_mm !== null && rack.depth_mm !== null
                    ? `${rack.width_mm} × ${rack.depth_mm} mm`
                    : "Not recorded"}
                </RecordRow>
                <RecordRow label="Cooling">{rack.cooling ?? "Not recorded"}</RecordRow>
                <RecordRow label="Units used">
                  {rack.used_units} of {rack.units}
                </RecordRow>
              </RecordSection>
              <RecordSection label="Power">
                <RecordRow label="Capacity">
                  {rack.power_capacity_kw !== null ? `${rack.power_capacity_kw} kW` : "Not recorded"}
                </RecordRow>
                <RecordRow label="Load">
                  {rack.power_used_kw !== null ? `${rack.power_used_kw} kW` : "Not recorded"}
                </RecordRow>
                {loadPct !== null ? (
                  <div
                    className="mt-1 h-2 w-full overflow-hidden rounded-full"
                    style={{ background: "var(--m-surface-2)" }}
                    role="img"
                    aria-label={`Power load ${loadPct} percent`}
                  >
                    <div
                      className="h-full"
                      style={{
                        width: `${Math.min(loadPct, 100)}%`,
                        background: loadPct > 85 ? "var(--m-blush)" : "var(--m-status-success-fg)",
                      }}
                    />
                  </div>
                ) : null}
              </RecordSection>
            </>
          }
          content={
            <div className="flex flex-col gap-4 pt-2">
<SurfaceHeader
                title={rack.name}
                meta={<StatusPill tone="neutral">{rack.units}U</StatusPill>}
                actions={
                  <div
            className="flex items-center gap-1 rounded-[10px] p-[3px]"
            style={{ background: "var(--m-surface-2)" }}
            role="group"
            aria-label="Rack face"
            >
            {(["front", "rear"] as const).map((side) => (
            <button
            key={side}
            type="button"
            aria-pressed={face === side}
            onClick={() => setFace(side)}
            className="h-8 rounded-[8px] px-3 text-[13px] font-semibold capitalize"
            style={{
            background: face === side ? "var(--m-surface)" : "transparent",
            color: face === side ? "var(--m-text)" : "var(--m-text-secondary)",
            }}
            >
            {side}
            </button>
            ))}
            </div>
                }
              />

              <section className="flex flex-wrap items-start gap-6" aria-label="Elevation">
                <RackElevation
                  units={rack.units}
                  devices={rack.devices}
                  face={face}
                  label={rack.name}
                />
                <div className="min-w-[280px] flex-1">
                  <h2 className="m-0 mb-2 text-[15px] font-semibold">Devices in this rack</h2>
                  <div
                    className="overflow-hidden rounded-[16px] border"
                    style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                  >
                    <table className="w-full border-collapse text-[13px]">
                      <thead>
                        <tr style={{ background: "var(--m-surface-2)" }}>
                          {["U", "Device", "Type", "Status"].map((head) => (
                            <th
                              key={head}
                              className={HEAD_CELL_CLASS}
                              style={{ color: "var(--m-text-muted)" }}
                            >
                              {head}
                            </th>
                          ))}
                        </tr>
                      </thead>
                      <tbody>
                        {rack.devices.length === 0 ? (
                          <tr>
                            <td colSpan={4} className="px-3 py-4" style={{ color: "var(--m-text-secondary)" }}>
                              Nothing placed yet.
                            </td>
                          </tr>
                        ) : (
                          rack.devices.map((device) => (
                            <tr
                              key={device.id}
                              style={{ borderTop: "1px solid var(--m-border)" }}
                            >
                              <td className="px-3 py-2 font-mono text-[12px] tabular-nums">
                                {device.unit_start}
                                {device.unit_height > 1
                                  ? `–${device.unit_start + device.unit_height - 1}`
                                  : ""}
                              </td>
                              <td className="px-3 py-2 font-semibold">
                                {device.name}
                                <span
                                  className="ml-2 text-[12px] font-normal"
                                  style={{ color: "var(--m-text-muted)" }}
                                >
                                  {device.face}
                                </span>
                              </td>
                              <td className="px-3 py-2 capitalize">{device.device_type}</td>
                              <td className="px-3 py-2 capitalize">{device.status}</td>
                            </tr>
                          ))
                        )}
                      </tbody>
                    </table>
                  </div>
                </div>
              </section>

              <form
                onSubmit={addDevice}
                className="flex flex-col gap-2 rounded-[16px] border p-3.5"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                aria-label="Place a device"
              >
                <h2 className="m-0 text-[15px] font-semibold">Place a device on the {face}</h2>
                {formError !== null ? (
                  <p className="m-0 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {formError}
                  </p>
                ) : null}
                <div className="flex flex-wrap items-end gap-2">
                  <Field label="Name">
                    <input name="name" required className={FIELD_CLASS} />
                  </Field>
                  <Field label="Type">
                    <select name="device_type" className={FIELD_CLASS} defaultValue="compute">
                      {DEVICE_TYPES.map((type) => (
                        <option key={type} value={type}>
                          {type}
                        </option>
                      ))}
                    </select>
                  </Field>
                  <Field label="Start U">
                    <input
                      name="unit_start"
                      type="number"
                      min={1}
                      max={rack.units}
                      required
                      className={FIELD_CLASS}
                    />
                  </Field>
                  <Field label="Height">
                    <input
                      name="unit_height"
                      type="number"
                      min={1}
                      max={12}
                      defaultValue={1}
                      className={FIELD_CLASS}
                    />
                  </Field>
                  <Field label="Model">
                    <input name="model" className={FIELD_CLASS} />
                  </Field>
                  <button
                    type="submit"
                    disabled={saving}
                    className="h-11 rounded-[10px] px-4 text-[13px] font-semibold disabled:opacity-60"
                    style={{ background: "var(--m-ink)", color: "var(--m-gold)" }}
                  >
                    {saving ? "Placing…" : "Place device"}
                  </button>
                </div>
              </form>
            </div>
          }
          summary={
            <>
              <SummaryCard title="Linked records">
                {rack.links.length === 0 ? (
                  <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                    Nothing linked yet. A rack links to its secrets, its switch documentation and
                    the site it stands in.
                  </p>
                ) : (
                  rack.links.map((link) => (
                    <RecordRow key={link.id} label={link.label ?? link.to_kind}>
                      {link.to_kind}
                    </RecordRow>
                  ))
                )}
              </SummaryCard>
              <SummaryCard title="Notes">
                <p className="text-[13px] leading-relaxed" style={{ color: "var(--m-text-secondary)" }}>
                  {rack.notes ?? "No notes on this rack."}
                </p>
              </SummaryCard>
            </>
          }
        />
      </div>
    </PageFrame>
  );
}

const FIELD_CLASS =
  "h-10 rounded-[10px] border px-2.5 text-[13px] outline-none focus-visible:outline-2";

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <label className="flex flex-col gap-1 text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
      <span>{label}</span>
      <span style={{ color: "var(--m-text)" }}>{children}</span>
    </label>
  );
}
