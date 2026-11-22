// /stock/ewaste — client e-waste custody.
//
// Purpose: receive a client's consignment with a signed intake sheet, read the
// weight balance per intake, record how each line left (recycled, destroyed,
// resold, returned, transferred), mark a data-bearing unit destroyed, and close
// the intake when the weight is accounted for.
//
// Non-purpose: no transport scheduling, no partner directory administration,
// no certificate upload (the document id and reference are what the record
// carries).

"use client";

import {
  Badge,
  Button,
  Card,
  Dialog,
  DialogActions,
  DialogContent,
  EmptyState,
  FieldError,
  FieldHint,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  SurfaceHeader,
  Table,
  Td,
  Th,
  Tr,
  formatDayKey,
} from "@marula-core/ui";
import type { components } from "@marula-core/sdk";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, useRequireAuth } from "../../../lib/api";
import { runEnvelopeMutation } from "../../../lib/envelope-write";
import { rejectionText } from "../../../lib/error-copy";

type Summary = components["schemas"]["IntakeSummary"];
type Detail = components["schemas"]["IntakeDetailResponse"];
type Line = components["schemas"]["EwasteIntakeLine"];
type Client = components["schemas"]["Client"];

const PATHWAYS = [
  { value: "recycled", label: "Recycled" },
  { value: "destroyed", label: "Destroyed" },
  { value: "resold", label: "Resold" },
  { value: "returned_to_client", label: "Returned to client" },
  { value: "transferred_to_partner", label: "Transferred to a partner" },
] as const;

function todayIso(): string {
  const now = new Date();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  return `${now.getFullYear()}-${month}-${day}`;
}

function grams(value: number): string {
  return `${value.toLocaleString()} g`;
}

type IntakeLineDraft = {
  category: string;
  lot_label: string;
  quantity: number;
  weight_g: number;
  data_bearing: boolean;
};

export default function EwastePage(): React.ReactNode {
  const auth = useRequireAuth();
  const [intakes, setIntakes] = useState<Summary[]>([]);
  const [clients, setClients] = useState<Client[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const [createOpen, setCreateOpen] = useState(false);
  const [clientId, setClientId] = useState("");
  const [receivedBy, setReceivedBy] = useState("");
  const [reference, setReference] = useState("");
  const [destructionRequired, setDestructionRequired] = useState(false);
  const [notes, setNotes] = useState("");
  const [lines, setLines] = useState<IntakeLineDraft[]>([]);
  const [category, setCategory] = useState("");
  const [lotLabel, setLotLabel] = useState("");
  const [quantity, setQuantity] = useState("1");
  const [lineWeight, setLineWeight] = useState("");
  const [lineDataBearing, setLineDataBearing] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [createBusy, setCreateBusy] = useState(false);
  const createSession = useRef(0);

  const [detail, setDetail] = useState<Detail | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const detailSeq = useRef(0);
  const [detailBusy, setDetailBusy] = useState(false);
  const [detailError, setDetailError] = useState<string | null>(null);

  const [disposeLine, setDisposeLine] = useState<Line | null>(null);
  const [pathway, setPathway] = useState("recycled");
  const [disposedWeight, setDisposedWeight] = useState("");
  const [partner, setPartner] = useState("");
  const [certificate, setCertificate] = useState("");
  const [disposeBusy, setDisposeBusy] = useState(false);
  const [disposeError, setDisposeError] = useState<string | null>(null);
  const disposeSession = useRef(0);

  const requestSeq = useRef(0);

  const refresh = useCallback(async (): Promise<void> => {
    const seq = ++requestSeq.current;
    setLoadError(null);
    try {
      const [intakeRes, clientRes] = await Promise.all([
        api.api.GET("/api/v1/ewaste/intakes", { params: { query: {} } }),
        api.api.GET("/api/v1/clients"),
      ]);
      if (seq !== requestSeq.current) return;
      if (intakeRes.data === undefined) {
        setLoadError("Could not load the e-waste register — try reloading.");
        return;
      }
      setIntakes(intakeRes.data);
      setClients((clientRes.data ?? []).map((c) => ({ ...c, name: c.name })));
    } catch {
      if (seq === requestSeq.current) {
        setLoadError("Could not load the e-waste register — try reloading.");
      }
    } finally {
      if (seq === requestSeq.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (auth.status !== "authed") return;
    setLoading(true);
    void refresh();
  }, [auth.status, refresh]);

  const clientName = useMemo(() => {
    const map = new Map(clients.map((c) => [c.id, c.name]));
    return (id: string): string => map.get(id) ?? id.slice(0, 8);
  }, [clients]);

  const openIntakes = intakes.filter((row) => row.intake.status === "open").length;
  const weightHeld = intakes.reduce((sum, row) => sum + row.remaining_g, 0);

  const addLine = (): void => {
    setCreateError(null);
    const qty = Number(quantity);
    const weight = Math.round(Number(lineWeight));
    if (category.trim() === "") {
      setCreateError("A line needs a category.");
      return;
    }
    if (!Number.isInteger(qty) || qty < 1) {
      setCreateError("A line needs a whole quantity.");
      return;
    }
    if (!Number.isFinite(weight) || weight <= 0) {
      setCreateError("A line needs a positive weight in grams.");
      return;
    }
    setLines((prev) => [
      ...prev,
      {
        category: category.trim(),
        lot_label: lotLabel.trim(),
        quantity: qty,
        weight_g: weight,
        data_bearing: lineDataBearing,
      },
    ]);
    setCategory("");
    setLotLabel("");
    setQuantity("1");
    setLineWeight("");
    setLineDataBearing(false);
  };

  const submitCreate = async (): Promise<void> => {
    const session = createSession.current;
    setCreateError(null);
    if (clientId === "") {
      setCreateError("Pick the client whose goods these are.");
      return;
    }
    if (receivedBy.trim() === "") {
      setCreateError("Name who received the consignment.");
      return;
    }
    setCreateBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Receive e-waste",
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({
        client_id: clientId,
        received_by_name: receivedBy.trim(),
        reference: reference.trim() === "" ? null : reference.trim(),
        data_destruction_required: destructionRequired,
        notes: notes.trim() === "" ? null : notes.trim(),
        lines: lines.map((line) => ({
          line_kind: "lot",
          category: line.category,
          lot_label: line.lot_label === "" ? null : line.lot_label,
          quantity: line.quantity,
          weight_g: line.weight_g,
          data_bearing: line.data_bearing,
        })),
        ...base,
      }),
      request: (client, body) =>
        client.POST("/api/v1/ewaste/intakes", {
          body: body as components["schemas"]["CreateIntakeRequest"],
        }),
    });
    if (!result.ok) {
      setCreateBusy(false);
      if (session === createSession.current) {
        setCreateError(rejectionText(result, "The intake was refused."));
      }
      return;
    }
    await refresh();
    if (session !== createSession.current) {
      setCreateBusy(false);
      return;
    }
    setCreateBusy(false);
    setCreateOpen(false);
    setLines([]);
    setReference("");
    setNotes("");
    setDestructionRequired(false);
    setNotice("Consignment received.");
  };

  /** Dismiss the dialog and invalidate any read still in flight. */
  const closeDetail = (): void => {
    detailSeq.current += 1;
    setDetailOpen(false);
    setDetail(null);
    setDetailError(null);
    setDetailBusy(false);
  };

  const openDetail = async (id: string): Promise<void> => {
    const seq = ++detailSeq.current;
    setDetailOpen(true);
    setDetailBusy(true);
    setDetailError(null);
    try {
      const res = await api.api.GET("/api/v1/ewaste/intakes/{id}", {
        params: { path: { id } },
      } satisfies { params: { path: { id: string } } });
      if (seq !== detailSeq.current) return; // a newer intake owns the dialog
      if (res.data === undefined) {
        setDetailError("Could not open that intake.");
        return;
      }
      setDetail(res.data);
    } catch {
      if (seq === detailSeq.current) setDetailError("Could not open that intake.");
    } finally {
      if (seq === detailSeq.current) setDetailBusy(false);
    }
  };

  const openDispose = (line: Line): void => {
    disposeSession.current += 1;
    setDisposeLine(line);
    setPathway("recycled");
    setDisposedWeight(String(line.weight_g - line.disposed_weight_g));
    setPartner("");
    setCertificate("");
    setDisposeError(null);
  };

  const submitDisposition = async (): Promise<void> => {
    const line = disposeLine;
    const current = detail;
    if (line === null || current === null) return;
    const session = disposeSession.current;
    const weight = Math.round(Number(disposedWeight));
    if (!Number.isFinite(weight) || weight <= 0) {
      setDisposeError("A disposition carries a positive weight.");
      return;
    }
    if (weight > line.weight_g - line.disposed_weight_g) {
      setDisposeError("That is more than this line has left.");
      return;
    }
    setDisposeBusy(true);
    const result = await runEnvelopeMutation({
      describe: "Record a disposition",
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({
        line_id: line.id,
        pathway,
        disposed_at: todayIso(),
        weight_g: weight,
        partner_name: partner.trim() === "" ? null : partner.trim(),
        certificate_reference: certificate.trim() === "" ? null : certificate.trim(),
        ...base,
      }),
      request: (client, body) =>
        client.POST("/api/v1/ewaste/intakes/{id}/dispositions", {
          params: { path: { id: current.intake.id } },
          body: body as components["schemas"]["RecordDispositionRequest"],
        }),
    });
    if (!result.ok) {
      setDisposeBusy(false);
      if (session === disposeSession.current) {
        setDisposeError(rejectionText(result, "The disposition was refused."));
      }
      return;
    }
    await openDetail(current.intake.id);
    await refresh();
    if (session !== disposeSession.current) {
      setDisposeBusy(false);
      return;
    }
    setDisposeBusy(false);
    setDisposeLine(null);
    setNotice("Disposition recorded.");
  };

  const markDestroyed = async (line: Line): Promise<void> => {
    const current = detail;
    if (current === null) return;
    setDetailBusy(true);
    setDetailError(null);
    const result = await runEnvelopeMutation({
      describe: "Complete data destruction",
      optimistic: () => {},
      revert: () => {},
      buildBody: (base) => ({ ...base, status: "completed", document_id: null }),
      request: (client, body) =>
        client.POST("/api/v1/ewaste/intakes/{id}/lines/{line_id}/destruction", {
          params: { path: { id: current.intake.id, line_id: line.id } },
          body: body as components["schemas"]["SetDestructionRequest"],
        }),
    });
    setDetailBusy(false);
    if (!result.ok) {
      setDetailError(rejectionText(result, "Data destruction was refused."));
      return;
    }
    await openDetail(current.intake.id);
    await refresh();
    setNotice("Data destruction recorded.");
  };

  const closeIntake = async (): Promise<void> => {
    const current = detail;
    if (current === null) return;
    setDetailBusy(true);
    setDetailError(null);
    const result = await runEnvelopeMutation({
      describe: "Close the intake",
      entityId: current.intake.id,
      optimistic: () => {},
      revert: () => {},
      // The version the detail read returned is the one the close is
      // conditional on; the mutation cache is not seeded by a read.
      buildBody: (base) => ({ ...base, version: current.version }),
      request: (client, body) =>
        client.POST("/api/v1/ewaste/intakes/{id}/close", {
          params: { path: { id: current.intake.id } },
          body: body as components["schemas"]["CloseIntakeRequest"],
        }),
    });
    setDetailBusy(false);
    if (!result.ok) {
      setDetailError(rejectionText(result, "The intake cannot close yet."));
      return;
    }
    await openDetail(current.intake.id);
    await refresh();
    setNotice("Intake closed.");
  };

  if (auth.status === "loading") return <Spinner />;

  return (
    <div className="grid gap-4">
      <SurfaceHeader
        title="E-waste custody"
        description={`Client goods received and tracked to a way out: ${openIntakes} open, ${grams(weightHeld)} still held.`}
        actions={
          <Button
            onClick={() => {
              createSession.current += 1;
              setCreateOpen(true);
            }}
            data-testid="ewaste-new"
          >
            Receive a consignment
          </Button>
        }
      />

      {notice !== null ? (
        <p role="status" className="text-sm" style={{ color: "var(--m-status-success-fg)" }}>
          {notice}
        </p>
      ) : null}
      {loadError !== null ? <FieldError>{loadError}</FieldError> : null}

      <Card>
        {loading ? (
          <div className="p-6">
            <Spinner />
          </div>
        ) : intakes.length === 0 ? (
          <EmptyState message="No consignments yet. Receive a client's e-waste to start the custody record." />
        ) : (
          <Table>
            <thead>
              <tr>
                <Th>Client</Th>
                <Th>Reference</Th>
                <Th>Received</Th>
                <Th className="text-right">Lines</Th>
                <Th className="text-right">In</Th>
                <Th className="text-right">Out</Th>
                <Th className="text-right">Held</Th>
                <Th>Status</Th>
                <Th />
              </tr>
            </thead>
            <tbody>
              {intakes.map((row) => (
                <Tr key={row.intake.id}>
                  <Td>{clientName(row.intake.client_id)}</Td>
                  <Td className="text-sm">{row.intake.reference ?? "—"}</Td>
                  <Td className="text-sm">{formatDayKey(row.intake.received_at.slice(0, 10), "day")}</Td>
                  <Td className="text-right tabular-nums text-sm">{row.line_count}</Td>
                  <Td className="text-right tabular-nums text-sm">{grams(row.total_in_g)}</Td>
                  <Td className="text-right tabular-nums text-sm">{grams(row.total_out_g)}</Td>
                  <Td className="text-right tabular-nums text-sm">{grams(row.remaining_g)}</Td>
                  <Td>
                    <Badge tone={row.intake.status === "closed" ? "neutral" : "success"}>
                      {row.intake.status}
                    </Badge>
                  </Td>
                  <Td className="text-right">
                    <Button
                      variant="secondary"
                      disabled={detailBusy}
                      onClick={() => void openDetail(row.intake.id)}
                    >
                      Open
                    </Button>
                  </Td>
                </Tr>
              ))}
            </tbody>
          </Table>
        )}
      </Card>

      {/* Create */}
      <Dialog
        open={createOpen}
        onOpenChange={(open) => {
          if (open) createSession.current += 1;
          setCreateOpen(open);
          if (!open) setCreateError(null);
        }}
      >
        <DialogContent
          title="Receive a consignment"
          description="What the client handed over, line by line, with the weight the record will balance against."
        >
          <div className="grid gap-3 p-1">
            <div>
              <Label>Client</Label>
              <Select value={clientId} onValueChange={setClientId}>
                <SelectTrigger className="w-full" aria-label="E-waste client" data-testid="ewaste-client">
                  <SelectValue placeholder="Pick a client" />
                </SelectTrigger>
                <SelectContent>
                  {clients.map((c) => (
                    <SelectItem key={c.id} value={c.id}>
                      {c.name}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex gap-2">
              <div className="flex-1">
                <Label htmlFor="ew-received-by">Received by</Label>
                <Input
                  id="ew-received-by"
                  value={receivedBy}
                  onChange={(e) => setReceivedBy(e.target.value)}
                  placeholder="Name on the intake sheet"
                  data-testid="ewaste-received-by"
                />
              </div>
              <div className="flex-1">
                <Label htmlFor="ew-reference">Reference</Label>
                <Input
                  id="ew-reference"
                  value={reference}
                  onChange={(e) => setReference(e.target.value)}
                  data-testid="ewaste-reference"
                />
              </div>
            </div>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={destructionRequired}
                onChange={(e) => setDestructionRequired(e.target.checked)}
                data-testid="ewaste-destruction-required"
              />
              The client requires data destruction before disposal
            </label>

            <div className="rounded-md border p-3" style={{ borderColor: "var(--m-border)" }}>
              <Label>Lines</Label>
              <p className="mt-1 text-xs" style={{ color: "var(--m-text-muted)" }}>
                One line per lot. The weights are what the balance closes against.
              </p>
              {lines.length > 0 ? (
                <ul className="mt-2 grid gap-1">
                  {lines.map((line, index) => (
                    <li key={`${line.category}-${index}`} className="flex items-center gap-2 text-xs">
                      <span className="min-w-0 flex-1 truncate">
                        {line.category}
                        {line.lot_label ? ` · ${line.lot_label}` : ""} · ×{line.quantity}
                      </span>
                      <span className="tabular-nums">{grams(line.weight_g)}</span>
                      <Button
                        variant="ghost"
                        onClick={() => setLines((prev) => prev.filter((_, i) => i !== index))}
                        aria-label={`Remove ${line.category}`}
                      >
                        Remove
                      </Button>
                    </li>
                  ))}
                </ul>
              ) : null}
              <div className="mt-2 grid gap-2">
                <div className="flex gap-2">
                  <div className="flex-1">
                    <Input
                      aria-label="Line category"
                      placeholder="Category (laptop, monitor…)"
                      value={category}
                      onChange={(e) => setCategory(e.target.value)}
                      data-testid="ewaste-line-category"
                    />
                  </div>
                  <div className="w-40">
                    <Input
                      aria-label="Lot label"
                      placeholder="Lot label"
                      value={lotLabel}
                      onChange={(e) => setLotLabel(e.target.value)}
                    />
                  </div>
                </div>
                <div className="flex gap-2">
                  <div className="w-24">
                    <Input
                      aria-label="Quantity"
                      type="number"
                      min="1"
                      value={quantity}
                      onChange={(e) => setQuantity(e.target.value)}
                    />
                  </div>
                  <div className="w-32">
                    <Input
                      aria-label="Weight in grams"
                      type="number"
                      min="1"
                      placeholder="grams"
                      value={lineWeight}
                      onChange={(e) => setLineWeight(e.target.value)}
                      data-testid="ewaste-line-weight"
                    />
                  </div>
                  <label className="flex flex-1 items-center gap-2 text-xs">
                    <input
                      type="checkbox"
                      checked={lineDataBearing}
                      onChange={(e) => setLineDataBearing(e.target.checked)}
                    />
                    Holds data
                  </label>
                  <Button variant="ghost" onClick={addLine}>
                    Add line
                  </Button>
                </div>
              </div>
            </div>
            <div>
              <Label htmlFor="ew-notes">Notes</Label>
              <Input id="ew-notes" value={notes} onChange={(e) => setNotes(e.target.value)} />
            </div>
            <FieldHint>The intake is a custody record: the goods never enter owned stock.</FieldHint>
            {createError !== null ? <FieldError>{createError}</FieldError> : null}
            <DialogActions>
              <Button variant="secondary" onClick={() => setCreateOpen(false)}>
                Cancel
              </Button>
              <Button disabled={createBusy} onClick={() => void submitCreate()} data-testid="ewaste-save">
                {createBusy ? "Receiving…" : "Receive"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>

      {/* Detail */}
      <Dialog
        open={detailOpen}
        onOpenChange={(open) => {
          if (!open) closeDetail();
        }}
      >
        <DialogContent title={detail === null ? "Consignment" : `Consignment ${detail.intake.reference ?? detail.intake.id.slice(0, 8)}`}>
          <div className="grid gap-3 overflow-auto p-1" style={{ maxHeight: "80vh" }}>
            {detail === null && detailError !== null ? <FieldError>{detailError}</FieldError> : null}
            {detail === null && detailError === null ? <Spinner /> : null}
            {detail !== null ? (
              <>
                <div className="flex flex-wrap gap-2 text-sm">
                  <Badge tone={detail.intake.status === "closed" ? "neutral" : "success"}>
                    {detail.intake.status}
                  </Badge>
                  <Badge tone="neutral">held {grams(detail.balance.remaining_g)}</Badge>
                  <Badge tone="neutral">in {grams(detail.balance.total_in_g)}</Badge>
                  <Badge tone="neutral">out {grams(detail.balance.total_out_g)}</Badge>
                </div>
                <Table>
                  <thead>
                    <tr>
                      <Th>Line</Th>
                      <Th className="text-right">Weight</Th>
                      <Th className="text-right">Disposed</Th>
                      <Th>Destruction</Th>
                      <Th />
                    </tr>
                  </thead>
                  <tbody>
                    {detail.lines.map((line) => {
                      const left = line.weight_g - line.disposed_weight_g;
                      return (
                        <Tr key={line.id}>
                          <Td className="text-sm">
                            {line.category}
                            {line.lot_label ? ` · ${line.lot_label}` : ""}
                          </Td>
                          <Td className="text-right tabular-nums text-sm">{grams(line.weight_g)}</Td>
                          <Td className="text-right tabular-nums text-sm">{grams(line.disposed_weight_g)}</Td>
                          <Td className="text-sm">
                            {line.data_bearing ? (
                              <span className="inline-flex items-center gap-1.5">
                                <Badge tone={line.destruction_status === "completed" ? "success" : "warning"}>
                                  {line.destruction_status}
                                </Badge>
                                {line.destruction_status !== "completed" &&
                                detail.intake.status === "open" ? (
                                  <Button variant="ghost" onClick={() => void markDestroyed(line)}>
                                    Mark destroyed
                                  </Button>
                                ) : null}
                              </span>
                            ) : (
                              "—"
                            )}
                          </Td>
                          <Td className="text-right">
                            {left > 0 && detail.intake.status === "open" ? (
                              <Button variant="secondary" onClick={() => openDispose(line)}>
                                Dispose {grams(left)}
                              </Button>
                            ) : null}
                          </Td>
                        </Tr>
                      );
                    })}
                  </tbody>
                </Table>

                <div>
                  <Label>Dispositions</Label>
                  {detail.dispositions.length === 0 ? (
                    <p className="mt-1 text-xs" style={{ color: "var(--m-text-muted)" }}>
                      Nothing has left yet.
                    </p>
                  ) : (
                    <Table>
                      <thead>
                        <tr>
                          <Th>Day</Th>
                          <Th>Pathway</Th>
                          <Th>Partner</Th>
                          <Th>Certificate</Th>
                          <Th className="text-right">Weight</Th>
                        </tr>
                      </thead>
                      <tbody>
                        {detail.dispositions.map((d) => (
                          <Tr key={d.id}>
                            <Td className="text-sm">{formatDayKey(d.disposed_at, "day")}</Td>
                            <Td className="text-sm">{d.pathway.replaceAll("_", " ")}</Td>
                            <Td className="text-sm">{d.partner_name ?? "—"}</Td>
                            <Td className="text-sm">{d.certificate_reference ?? "—"}</Td>
                            <Td className="text-right tabular-nums text-sm">{grams(d.weight_g)}</Td>
                          </Tr>
                        ))}
                      </tbody>
                    </Table>
                  )}
                </div>

                {detailError !== null ? <FieldError>{detailError}</FieldError> : null}
                <DialogActions>
                  <Button variant="secondary" onClick={closeDetail}>
                    Close
                  </Button>
                  {detail.intake.status === "open" ? (
                    <Button disabled={detailBusy} onClick={() => void closeIntake()} data-testid="ewaste-close">
                      {detailBusy ? "Closing…" : "Close the intake"}
                    </Button>
                  ) : null}
                </DialogActions>
              </>
            ) : (
              <Spinner />
            )}
          </div>
        </DialogContent>
      </Dialog>

      {/* Disposition */}
      <Dialog
        open={disposeLine !== null}
        onOpenChange={(open) => {
          if (!open) setDisposeLine(null);
        }}
      >
        <DialogContent title="Record a disposition" description="How these grams left the building.">
          <div className="grid gap-3 p-1">
            <div>
              <Label>Pathway</Label>
              <Select value={pathway} onValueChange={setPathway}>
                <SelectTrigger className="w-full" aria-label="Disposition pathway" data-testid="ewaste-pathway">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {PATHWAYS.map((p) => (
                    <SelectItem key={p.value} value={p.value}>
                      {p.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="flex gap-2">
              <div className="w-36">
                <Label htmlFor="ew-weight">Weight (g)</Label>
                <Input
                  id="ew-weight"
                  type="number"
                  min="1"
                  value={disposedWeight}
                  onChange={(e) => setDisposedWeight(e.target.value)}
                  data-testid="ewaste-dispose-weight"
                />
              </div>
              <div className="flex-1">
                <Label htmlFor="ew-partner">Partner (recycled or transferred)</Label>
                <Input id="ew-partner" value={partner} onChange={(e) => setPartner(e.target.value)} />
              </div>
            </div>
            <div>
              <Label htmlFor="ew-certificate">Certificate reference</Label>
              <Input
                id="ew-certificate"
                value={certificate}
                onChange={(e) => setCertificate(e.target.value)}
                placeholder="Required for recycled, destroyed and transferred"
                data-testid="ewaste-certificate"
              />
            </div>
            {disposeError !== null ? <FieldError>{disposeError}</FieldError> : null}
            <DialogActions>
              <Button variant="secondary" onClick={() => setDisposeLine(null)}>
                Cancel
              </Button>
              <Button disabled={disposeBusy} onClick={() => void submitDisposition()} data-testid="ewaste-dispose-save">
                {disposeBusy ? "Recording…" : "Record"}
              </Button>
            </DialogActions>
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}
