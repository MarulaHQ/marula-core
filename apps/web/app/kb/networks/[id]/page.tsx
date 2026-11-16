"use client";
// /kb/networks/[id] — IPAM for one network: subnets, VLANs, reservations.
//
// Purpose: the pack's K06 anatomy without the site record (that is a separate
// surface this lane did not build — see the report's gap list). The internet
// circuit, the VLAN table and the address reservations are here; the doors,
// alarm codes and photos belong to the site.

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
import {
  knowledgeApi,
  type KnowledgeRequestError,
  type NetworkDetail,
} from "../../../../components/knowledge/knowledge-api";
import {
  RecordFrame,
  RecordRow,
  RecordSection,
  SummaryCard,
} from "../../../../components/records/record-frame";

export default function NetworkPage() {
  useRequireAuth();
  const params = useParams<{ id: string }>();
  const networkId = params.id;
  const [network, setNetwork] = useState<NetworkDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const load = useCallback(() => {
    if (networkId === undefined) return;
    setError(null);
    knowledgeApi
      .get<NetworkDetail>(`/api/v1/knowledge/networks/${networkId}`)
      .then(setNetwork)
      .catch((e: KnowledgeRequestError) => setError(e.message));
  }, [networkId]);

  useEffect(load, [load]);

  async function addSubnet(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const data = new FormData(form);
    setFormError(null);
    setSaving(true);
    try {
      await knowledgeApi.post(`/api/v1/knowledge/networks/${networkId}/subnets`, {
        cidr: String(data.get("cidr") ?? ""),
        vlan_id: data.get("vlan_id") === "" ? null : Number(data.get("vlan_id")),
        name: String(data.get("name") ?? "") || null,
        gateway: String(data.get("gateway") ?? "") || null,
        used_for: String(data.get("used_for") ?? "") || null,
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
          <ErrorNote title="The network did not load.">{error}</ErrorNote>
        </div>
      </PageFrame>
    );
  }

  if (network === null) {
    return (
      <PageFrame>
        <div className="pt-6">
          <SkeletonRows rows={7} sentence="Loading the network" columns={3} />
        </div>
      </PageFrame>
    );
  }

  const reserved = network.reservations.filter((r) => r.status !== "deprecated").length;

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
            href={`/kb/networks?client_id=${network.client_id}`}
            className="underline underline-offset-2"
            style={{ color: "var(--m-text-secondary)" }}
          >
            Networks
          </Link>
          <span aria-hidden="true" style={{ color: "var(--m-text-muted)" }}>
            /
          </span>
          <span className="font-semibold">{network.name}</span>
        </nav>

        <RecordFrame
          context={
            <>
              <RecordSection label="Network">
                <p className="text-[18px] font-semibold leading-tight">{network.name}</p>
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {network.kind.toUpperCase()}
                  {network.vlan_zone !== null ? ` · ${network.vlan_zone}` : ""}
                </p>
              </RecordSection>
              <RecordSection label="Addressing">
                <RecordRow label="Gateway">{network.gateway ?? "Not recorded"}</RecordRow>
                <RecordRow label="DNS">{network.dns ?? "Not recorded"}</RecordRow>
                <RecordRow label="Subnets">{network.subnets.length}</RecordRow>
                <RecordRow label="Reserved">{reserved}</RecordRow>
              </RecordSection>
              <RecordSection label="Site">
                <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                  {network.site_id === null
                    ? "Not tied to a site yet. A site record carries the access hours, the doors and the contacts."
                    : "Open the site record from the client's sites list."}
                </p>
              </RecordSection>
            </>
          }
          content={
            <div className="flex flex-col gap-4 pt-2">
<SurfaceHeader
          title={network.name}
          meta={<StatusPill tone="neutral">{network.kind.toUpperCase()}</StatusPill>}
        />

              <section aria-label="Subnets and VLANs" className="flex flex-col gap-2">
                <h2 className="m-0 text-[15px] font-semibold">Subnets and VLANs</h2>
                <div
                  className="overflow-hidden rounded-[16px] border"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                >
                  <table className="w-full border-collapse text-[13px]">
                    <thead>
                      <tr style={{ background: "var(--m-surface-2)" }}>
                        {["VLAN", "Subnet", "Name", "Gateway", "Used for", "DHCP"].map((head) => (
                          <th key={head} className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
                            {head}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {network.subnets.length === 0 ? (
                        <tr>
                          <td colSpan={6} className="px-3 py-4" style={{ color: "var(--m-text-secondary)" }}>
                            No subnets yet. Add one below and its addresses can be reserved.
                          </td>
                        </tr>
                      ) : (
                        network.subnets.map((subnet) => (
                          <tr key={subnet.id} style={{ borderTop: "1px solid var(--m-border)" }}>
                            <td className="px-3 py-2 font-mono text-[12px] tabular-nums">
                              {subnet.vlan_id ?? "—"}
                            </td>
                            <td className="px-3 py-2 font-mono text-[12px]">{subnet.cidr}</td>
                            <td className="px-3 py-2 font-semibold">{subnet.name ?? "—"}</td>
                            <td className="px-3 py-2 font-mono text-[12px]">{subnet.gateway ?? "—"}</td>
                            <td className="px-3 py-2">{subnet.used_for ?? "—"}</td>
                            <td className="px-3 py-2">{subnet.dhcp ? "Yes" : "No"}</td>
                          </tr>
                        ))
                      )}
                    </tbody>
                  </table>
                </div>
              </section>

              <section aria-label="Address reservations" className="flex flex-col gap-2">
                <h2 className="m-0 text-[15px] font-semibold">Address reservations</h2>
                <div
                  className="overflow-hidden rounded-[16px] border"
                  style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                >
                  <table className="w-full border-collapse text-[13px]">
                    <thead>
                      <tr style={{ background: "var(--m-surface-2)" }}>
                        {["Address", "Hostname", "Status", "Notes"].map((head) => (
                          <th key={head} className={HEAD_CELL_CLASS} style={{ color: "var(--m-text-muted)" }}>
                            {head}
                          </th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {network.reservations.length === 0 ? (
                        <tr>
                          <td colSpan={4} className="px-3 py-4" style={{ color: "var(--m-text-secondary)" }}>
                            Nothing reserved. A reservation is how a static address stops being a
                            surprise.
                          </td>
                        </tr>
                      ) : (
                        network.reservations.map((reservation) => (
                          <tr key={reservation.id} style={{ borderTop: "1px solid var(--m-border)" }}>
                            <td className="px-3 py-2 font-mono text-[12px]">{reservation.address}</td>
                            <td className="px-3 py-2 font-semibold">{reservation.hostname ?? "—"}</td>
                            <td className="px-3 py-2 capitalize">{reservation.status}</td>
                            <td className="px-3 py-2">{reservation.notes ?? "—"}</td>
                          </tr>
                        ))
                      )}
                    </tbody>
                  </table>
                </div>
              </section>

              <form
                onSubmit={addSubnet}
                className="flex flex-col gap-2 rounded-[16px] border p-3.5"
                style={{ borderColor: "var(--m-border)", background: "var(--m-surface)" }}
                aria-label="Add a subnet"
              >
                <h2 className="m-0 text-[15px] font-semibold">Add a subnet</h2>
                {formError !== null ? (
                  <p className="m-0 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
                    {formError}
                  </p>
                ) : null}
                <div className="flex flex-wrap items-end gap-2">
                  <Field label="CIDR">
                    <input name="cidr" required placeholder="10.20.30.0/24" className={FIELD_CLASS} />
                  </Field>
                  <Field label="VLAN">
                    <input name="vlan_id" type="number" min={1} max={4094} className={FIELD_CLASS} />
                  </Field>
                  <Field label="Name">
                    <input name="name" className={FIELD_CLASS} />
                  </Field>
                  <Field label="Gateway">
                    <input name="gateway" className={FIELD_CLASS} />
                  </Field>
                  <Field label="Used for">
                    <input name="used_for" className={FIELD_CLASS} />
                  </Field>
                  <button
                    type="submit"
                    disabled={saving}
                    className="h-11 rounded-[10px] px-4 text-[13px] font-semibold disabled:opacity-60"
                    style={{ background: "var(--m-ink)", color: "var(--m-gold)" }}
                  >
                    {saving ? "Adding…" : "Add subnet"}
                  </button>
                </div>
              </form>
            </div>
          }
          summary={
            <>
              <SummaryCard title="Linked records">
                {network.links.length === 0 ? (
                  <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
                    Nothing linked. Link the firewall, the switch and the site.
                  </p>
                ) : (
                  network.links.map((link) => (
                    <RecordRow key={link.id} label={link.label ?? link.to_kind}>
                      {link.to_kind}
                    </RecordRow>
                  ))
                )}
              </SummaryCard>
              <SummaryCard title="Notes">
                <p className="text-[13px] leading-relaxed" style={{ color: "var(--m-text-secondary)" }}>
                  {network.notes ?? "No notes on this network."}
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
  "h-10 w-36 rounded-[10px] border px-2.5 text-[13px] outline-none focus-visible:outline-2";

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
