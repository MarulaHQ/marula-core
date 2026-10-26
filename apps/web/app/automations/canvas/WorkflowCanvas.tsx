"use client";
// Workflow canvas — the power tier (Wave 43 lane 16, `p43-workflow-canvas`).
//
// Purpose: a node/edge editor over the SAME rule row the simple-tier rule
// builder edits (ADR 0016 decision 3). It edits `definition` — the multi-step
// workflow document the executor runs (`core-domain::workflow_document`) — using
// `@xyflow/react` for the graph substrate and the lane-13 registry for
// everything a node is: the palette, the per-node schema behind the properties
// panel, and the node family. A trigger is not a node (the document refuses
// one); it lives on the rule row and is shown as the entry.
//
// Composed from the design reference of record: the committed pack's
// `workflow-builder.html` (states W01–W09) supplies the shape — header, dotted
// canvas, node cards, inspector, library, validation strip, steps view — but
// every colour is a `--m-*` token (canvas.css), and the controls are
// `packages/ui` primitives. Non-purpose: no second component system, no script
// node, no second way to test (the rule dry-run is reused).
import {
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  Panel,
  Position,
  ReactFlow,
  ReactFlowProvider,
  addEdge,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Connection,
  type Edge,
  type Node,
  type NodeProps,
  type NodeTypes,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Badge, Button, EmptyState, Spinner } from "@marula-core/ui";
import Link from "next/link";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, useRequireAuth } from "../../../lib/api";
import {
  fetchCatalogue,
  listWorkflowDocuments,
  createWorkflowDocument,
  updateWorkflowDocument,
  testRunRule,
  type NodeCatalogueView,
  type RegistryNodeView,
  type WorkflowDocumentView,
} from "../../../lib/workflow-canvas/api";
import {
  emptyDocument,
  parseDocument,
  summariseParams,
  validateDocument,
  stableEdgeKey,
  edgeKey,
  type DocumentIssue,
  type WorkflowDocumentDoc,
  type WorkflowNodeDoc,
} from "../../../lib/workflow-canvas/document";
import { deriveDocumentFromRule, describeRule, type SimpleRule } from "../../../lib/workflow-canvas/derive";
import { layoutDocument, type NodePosition } from "../../../lib/workflow-canvas/layout";
import { schemaFields, seedParams, validateAgainstSchema, type JsonSchema } from "../../../lib/workflow-canvas/schema";
import "./canvas.css";

// --- React Flow node shape ---------------------------------------------------

interface StepNodeData extends Record<string, unknown> {
  nodeType: string;
  params: Record<string, unknown>;
  label: string;
  family: RegistryNodeView["family"];
  idempotent: boolean;
  invalid: boolean;
}

type StepNodeType = Node<StepNodeData, "step">;

const FAMILY_LABEL: Record<RegistryNodeView["family"], string> = {
  trigger: "Starts the workflow",
  decision: "Decision",
  condition: "Condition",
  action: "Action",
  control: "Flow",
  value: "Value",
};

/** One node card, built from the mockup's `.nd` anatomy + ui primitives. */
function StepNode({ data, selected }: NodeProps<StepNodeType>) {
  const summary = Object.keys(data.params).length
    ? summariseParams({ id: "", node_type: data.nodeType, params: data.params })
    : "";
  const trigger = data.family === "trigger";
  return (
    <div
      className={`wfnd${selected ? " sel" : ""}${data.invalid ? " invalid" : ""}`}
      aria-label={`Step: ${data.label}. ${FAMILY_LABEL[data.family]}.${data.invalid ? " Needs attention." : ""}`}
    >
      <Handle type="target" position={Position.Left} className="wf-handle" />
      <div className="wfnd-h">
        <span className={`wfnd-i${trigger ? " trig" : ""}`} aria-hidden="true">
          {data.family.slice(0, 1).toUpperCase()}
        </span>
        <div className="wfnd-t">
          <b>{data.label}</b>
          <span>
            {FAMILY_LABEL[data.family]}
            {!data.idempotent ? " · not safe to repeat" : ""}
          </span>
        </div>
        {data.invalid ? (
          <Badge tone="danger">Fix</Badge>
        ) : null}
      </div>
      {summary ? (
        <div className="wfnd-b">
          <span className="wfnd-outs">{summary}</span>
        </div>
      ) : null}
      <Handle type="source" position={Position.Right} className="wf-handle" />
    </div>
  );
}

const NODE_TYPES: NodeTypes = { step: StepNode };

// --- document ↔ React Flow ---------------------------------------------------

function positionsLookup(
  positions: Record<string, NodePosition>,
): (id: string) => NodePosition {
  return (id) => positions[id] ?? { x: 0, y: 0 };
}

function docToFlow(
  doc: WorkflowDocumentDoc,
  byId: Map<string, RegistryNodeView>,
  positionOf: (id: string) => NodePosition,
  issuesByNode: Map<string, string[]>,
): { nodes: StepNodeType[]; edges: Edge[] } {
  const nodes: StepNodeType[] = doc.nodes.map((n) => {
    const reg = byId.get(n.node_type);
    return {
      id: n.id,
      type: "step",
      position: positionOf(n.id),
      data: {
        nodeType: n.node_type,
        params: n.params,
        label: reg?.label ?? n.node_type,
        family: reg?.family ?? "action",
        idempotent: reg?.idempotent ?? true,
        invalid: (issuesByNode.get(n.id)?.length ?? 0) > 0,
      },
    };
  });
  const edges: Edge[] = doc.edges.map((e) => ({
    id: stableEdgeKey(e),
    source: e.from,
    target: e.to,
    label: e.label,
    markerEnd: { type: MarkerType.ArrowClosed, color: "var(--m-input)" },
    ariaLabel: e.label ? `Edge labelled ${e.label}` : "Edge",
  }));
  return { nodes, edges };
}

function flowToDoc(nodes: StepNodeType[], edges: Edge[], loopMaxDepth: number): WorkflowDocumentDoc {
  return {
    version: "1",
    nodes: nodes.map<WorkflowNodeDoc>((n) => ({
      id: n.id,
      node_type: n.data.nodeType,
      params: n.data.params,
    })),
    edges: edges.map((e) => ({
      from: e.source,
      to: e.target,
      ...(typeof e.label === "string" && e.label ? { label: e.label } : {}),
    })),
    loop_max_depth: loopMaxDepth,
  };
}

interface Snapshot {
  nodes: StepNodeType[];
  edges: Edge[];
}

// --- the surface -------------------------------------------------------------

interface WorkflowCanvasProps {
  /** A simple rule to promote into the canvas, by id. */
  ruleId?: string;
}

export default function WorkflowCanvas({ ruleId }: WorkflowCanvasProps) {
  const { status } = useRequireAuth();
  const [catalogue, setCatalogue] = useState<NodeCatalogueView | null>(null);
  const [documents, setDocuments] = useState<WorkflowDocumentView[]>([]);
  const [meta, setMeta] = useState<{ id?: string; name: string; trigger: string; revision: number } | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [draftIssues, setDraftIssues] = useState<DocumentIssue[]>([]);
  const [name, setName] = useState("");
  const [trigger, setTrigger] = useState("ticket.created");
  const [subject, setSubject] = useState<"ticket" | "board">("ticket");
  const [view, setView] = useState<"canvas" | "steps">("canvas");
  const [libraryOpen, setLibraryOpen] = useState(false);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [dryRun, setDryRun] = useState<null | { text: string; clean: boolean }>(null);
  const [revision, setRevision] = useState(0);

  const [nodes, setNodes, onNodesChange] = useNodesState<StepNodeType>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const wrapper = useRef<HTMLDivElement | null>(null);
  const past = useRef<Snapshot[]>([]);
  const future = useRef<Snapshot[]>([]);
  const clipboard = useRef<Snapshot | null>(null);
  const loadKeyRef = useRef<string | null>(null);
  const [, forceTick] = useState(0);

  const byId = useMemo(() => {
    const map = new Map<string, RegistryNodeView>();
    for (const n of catalogue?.nodes ?? []) map.set(n.id, n);
    return map;
  }, [catalogue]);

  const palette = useMemo(
    () => (catalogue?.nodes ?? []).filter((n) => n.family !== "trigger"),
    [catalogue],
  );

  // --- history ---------------------------------------------------------------

  const snapshot = useCallback(
    (): Snapshot => ({ nodes: nodes.map((n) => ({ ...n, data: { ...n.data } })), edges: edges.map((e) => ({ ...e })) }),
    [nodes, edges],
  );

  const pushHistory = useCallback(() => {
    past.current = [...past.current.slice(-49), snapshot()];
    future.current = [];
    forceTick((t) => t + 1);
  }, [snapshot]);

  const restore = useCallback(
    (snap: Snapshot) => {
      setNodes(snap.nodes);
      setEdges(snap.edges);
    },
    [setNodes, setEdges],
  );

  const undo = useCallback(() => {
    const prev = past.current.pop();
    if (!prev) return;
    future.current = [...future.current, snapshot()];
    restore(prev);
    forceTick((t) => t + 1);
  }, [restore, snapshot]);

  const redo = useCallback(() => {
    const next = future.current.pop();
    if (!next) return;
    past.current = [...past.current, snapshot()];
    restore(next);
    forceTick((t) => t + 1);
  }, [restore, snapshot]);

  // --- load ------------------------------------------------------------------

  // One load per (status, ruleId). The catalogue is fetched once and the
  // document is applied from it — the apply step takes the fetched catalogue
  // directly rather than the memoised one, so setting `catalogue` cannot
  // re-trigger the effect (that churn was an infinite fetch loop).
  useEffect(() => {
    if (status !== "authed") return;
    const key = ruleId ?? "__first__";
    if (loadKeyRef.current === key) return;
    loadKeyRef.current = key;
    let cancelled = false;
    let settled = false;

    const apply = async (
      doc: WorkflowDocumentDoc,
      nextMeta: typeof meta,
      initialIssues: DocumentIssue[],
      cat: NodeCatalogueView,
    ) => {
      const localById = new Map<string, RegistryNodeView>();
      for (const n of cat.nodes) localById.set(n.id, n);
      const positions = await layoutDocument(doc);
      const issuesByNode = new Map<string, string[]>();
      for (const issue of initialIssues) {
        if (!issue.nodeId) continue;
        issuesByNode.set(issue.nodeId, [...(issuesByNode.get(issue.nodeId) ?? []), issue.message]);
      }
      if (cancelled) return;
      const flow = docToFlow(doc, localById, positionsLookup(positions), issuesByNode);
      setNodes(flow.nodes);
      setEdges(flow.edges);
      past.current = [];
      future.current = [];
      setMeta(nextMeta);
      setDraftIssues(initialIssues);
    };

    (async () => {
      setLoading(true);
      try {
        const cat = await fetchCatalogue();
        if (cancelled) return;
        setCatalogue(cat);
        const docs = await listWorkflowDocuments();
        if (cancelled) return;
        setDocuments(docs);

        if (ruleId) {
          // Promote a simple rule: derive a document, keep the rule's fields.
          const { data } = await api.api.GET("/api/v1/automation-rules");
          const rules = (data as unknown as SimpleRule[] | undefined) ?? [];
          const rule = rules.find((r) => (r as unknown as { id?: string }).id === ruleId);
          if (rule) {
            const derived = deriveDocumentFromRule(rule);
            setName(`Canvas — ${describeRule(rule).slice(0, 40)}`);
            setTrigger(rule.trigger_event);
            setSubject(rule.subject_kind === "board" ? "board" : "ticket");
            await apply(derived.document, null, derived.issues, cat);
          } else {
            await apply(emptyDocument(), null, [], cat);
          }
        } else if (docs.length > 0 && docs[0]) {
          const first = docs[0];
          setName(first.name);
          setTrigger(first.trigger_event);
          setSubject(first.subject_kind === "board" ? "board" : "ticket");
          await apply(
            parseDocument(first.definition) ?? emptyDocument(),
            { id: first.id, name: first.name, trigger: first.trigger_event, revision: first.revision },
            [],
            cat,
          );
        } else {
          await apply(emptyDocument(), null, [], cat);
        }
        setRevision((r) => r + 1);
      } catch (e) {
        if (!cancelled) setLoadError(e instanceof Error ? e.message : "Could not open the canvas.");
      } finally {
        if (!cancelled) {
          settled = true;
          setLoading(false);
          setSaveState("idle");
        }
      }
    })();

    return () => {
      cancelled = true;
      // A remount before the load settled (dev StrictMode) must be allowed to
      // retry; a settled load keeps its guard so it never re-fetches.
      if (!settled) loadKeyRef.current = null;
    };
  }, [status, ruleId, setNodes, setEdges]);

  // --- validation ------------------------------------------------------------

  const issues = useMemo<DocumentIssue[]>(() => {
    const doc = flowToDoc(nodes, edges, 1);
    const known = new Set(palette.map((n) => n.id));
    const base = [...draftIssues, ...validateDocument(doc, { knownNodeTypes: known })];
    // Per-node param validation against the registry schema, so a missing
    // required field is a badge on the card, not a save-time surprise.
    for (const n of nodes) {
      const reg = byId.get(n.data.nodeType);
      if (!reg) continue;
      const violations = validateAgainstSchema(reg.input_schema as JsonSchema, n.data.params);
      for (const v of violations) {
        base.push({
          nodeId: n.id,
          message: `${v.path || "params"} ${v.message}`,
          severity: "error",
        });
      }
    }
    return base;
  }, [nodes, edges, draftIssues, palette, byId]);

  const issuesByNode = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const issue of issues) {
      if (!issue.nodeId) continue;
      map.set(issue.nodeId, [...(map.get(issue.nodeId) ?? []), issue.message]);
    }
    return map;
  }, [issues]);

  // Reflect validation onto the cards without a re-layout. Returns the SAME
  // array when nothing changed, so the effect that recomputes `issues` from
  // `nodes` cannot ping-pong with this one (React error 185).
  useEffect(() => {
    setNodes((current) => {
      let changed = false;
      const next = current.map((n) => {
        const invalid = (issuesByNode.get(n.id)?.length ?? 0) > 0;
        if (n.data.invalid === invalid) return n;
        changed = true;
        return { ...n, data: { ...n.data, invalid } };
      });
      return changed ? next : current;
    });
  }, [issuesByNode, setNodes]);

  const errorCount = issues.filter((i) => i.severity === "error").length;

  // --- mutation helpers ------------------------------------------------------

  const onConnect = useCallback(
    (connection: Connection) => {
      pushHistory();
      setEdges((eds) => addEdge({ ...connection, markerEnd: { type: MarkerType.ArrowClosed } }, eds));
    },
    [pushHistory, setEdges],
  );

  const addStep = useCallback(
    (reg: RegistryNodeView) => {
      pushHistory();
      const id = `${reg.id.replace(/\./g, "_")}_${Math.random().toString(36).slice(2, 6)}`;
      const params = seedParams(reg.input_schema as JsonSchema);
      setNodes((current) => [
        ...current,
        {
          id,
          type: "step",
          position: { x: 80 + current.length * 24, y: 120 + current.length * 40 },
          data: {
            nodeType: reg.id,
            params,
            label: reg.label,
            family: reg.family,
            idempotent: reg.idempotent,
            invalid: false,
          },
        },
      ]);
      setSelected(id);
      setLibraryOpen(false);
    },
    [pushHistory, setNodes],
  );

  const updateParams = useCallback(
    (nodeId: string, key: string, value: unknown) => {
      pushHistory();
      setNodes((current) =>
        current.map((n) =>
          n.id === nodeId ? { ...n, data: { ...n.data, params: { ...n.data.params, [key]: value } } } : n,
        ),
      );
    },
    [pushHistory, setNodes],
  );

  const removeNode = useCallback(
    (nodeId: string) => {
      pushHistory();
      setNodes((current) => current.filter((n) => n.id !== nodeId));
      setEdges((current) => current.filter((e) => e.source !== nodeId && e.target !== nodeId));
      setSelected((s) => (s === nodeId ? null : s));
    },
    [pushHistory, setNodes, setEdges],
  );

  const connectTo = useCallback(
    (from: string, to: string) => {
      if (from === to) return;
      pushHistory();
      setEdges((current) =>
        addEdge(
          { id: edgeKey(from, to), source: from, target: to, markerEnd: { type: MarkerType.ArrowClosed } },
          current,
        ),
      );
    },
    [pushHistory, setEdges],
  );

  const autoLayout = useCallback(async () => {
    pushHistory();
    const doc = flowToDoc(nodes, edges, 1);
    const positions = await layoutDocument(doc);
    const positionOf = positionsLookup(positions);
    setNodes((current) => current.map((n) => ({ ...n, position: positionOf(n.id) })));
  }, [nodes, edges, pushHistory, setNodes]);

  const copySelection = useCallback(() => {
    const sel = nodes.filter((n) => n.selected);
    if (sel.length === 0) return;
    const ids = new Set(sel.map((n) => n.id));
    clipboard.current = {
      nodes: sel.map((n) => ({ ...n, data: { ...n.data } })),
      edges: edges.filter((e) => ids.has(e.source) && ids.has(e.target)).map((e) => ({ ...e })),
    };
  }, [nodes, edges]);

  const paste = useCallback(() => {
    const clip = clipboard.current;
    if (!clip || clip.nodes.length === 0) return;
    pushHistory();
    const remap = new Map<string, string>();
    const fresh = clip.nodes.map((n) => {
      const id = `${n.id}_copy${Math.random().toString(36).slice(2, 5)}`;
      remap.set(n.id, id);
      return { ...n, id, selected: true, position: { x: n.position.x + 40, y: n.position.y + 40 } };
    });
    setNodes((current) => [...current.map((n) => ({ ...n, selected: false })), ...fresh]);
    setEdges((current) => [
      ...current,
      ...clip.edges.map((e) => ({
        ...e,
        id: edgeKey(remap.get(e.source) ?? e.source, remap.get(e.target) ?? e.target, typeof e.label === "string" ? e.label : null),
        source: remap.get(e.source) ?? e.source,
        target: remap.get(e.target) ?? e.target,
      })),
    ]);
  }, [pushHistory, setNodes, setEdges]);

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (mod && event.key.toLowerCase() === "z" && !event.shiftKey) {
        event.preventDefault();
        undo();
      } else if ((mod && event.key.toLowerCase() === "y") || (mod && event.shiftKey && event.key.toLowerCase() === "z")) {
        event.preventDefault();
        redo();
      } else if (mod && event.key.toLowerCase() === "c") {
        copySelection();
      } else if (mod && event.key.toLowerCase() === "v") {
        paste();
      }
    },
    [undo, redo, copySelection, paste],
  );

  // --- save ------------------------------------------------------------------

  const save = useCallback(async () => {
    setSaveState("saving");
    setSaveError(null);
    const definition = flowToDoc(nodes, edges, 1);
    try {
      const saved = meta?.id
        ? await updateWorkflowDocument(meta.id, {
            name,
            trigger_event: trigger,
            subject_kind: subject,
            definition,
          })
        : await createWorkflowDocument({
            name,
            trigger_event: trigger,
            subject_kind: subject,
            definition,
          });
      setMeta({ id: saved.id, name: saved.name, trigger: saved.trigger_event, revision: saved.revision });
      setSaveState("saved");
      setDocuments((current) => {
        const rest = current.filter((d) => d.id !== saved.id);
        return [...rest, saved];
      });
    } catch (e) {
      setSaveState("error");
      setSaveError(e instanceof Error ? e.message : "Could not save.");
    }
  }, [nodes, edges, meta, name, trigger, subject]);

  const runPreflight = useCallback(async () => {
    if (errorCount > 0) {
      setDryRun({ text: `${errorCount} problem${errorCount === 1 ? "" : "s"} to fix before this can run.`, clean: false });
      return;
    }
    if (!meta?.id) {
      setDryRun({
        text: "Save first — the document is validated by the executor's own check when it saves.",
        clean: true,
      });
      return;
    }
    try {
      const result = await testRunRule(meta.id);
      setDryRun({
        text: `Test run sampled ${result.sampled_count} records; ${result.matched_count} would match. Nothing was applied.`,
        clean: true,
      });
    } catch (e) {
      setDryRun({
        text:
          e instanceof Error
            ? `${e.message} A multi-step workflow is validated on save rather than by the simple-tier dry run.`
            : "Test run failed.",
        clean: false,
      });
    }
  }, [errorCount, meta]);

  if (status === "loading" || loading) {
    return (
      <div className="wf" aria-busy="true">
        <div className="wf-center">
          <Spinner />
        </div>
      </div>
    );
  }

  if (loadError) {
    return (
      <div className="wf">
        <EmptyState message={`The canvas could not open. ${loadError}`} />
      </div>
    );
  }

  const selectedNode = nodes.find((n) => n.id === selected) ?? null;

  return (
    <div className="wf">
      <a href="#wf-inspector" className="wf-skip">
        Skip to the step settings
      </a>
      <header className="wf-h">
        <div>
          <div className="wf-crumb">
            <Link href="/automations">Automations</Link> › Workflows
          </div>
          <div className="wf-title-row">
            <h1>{name || "Untitled workflow"}</h1>
            <Badge tone={meta?.id ? "neutral" : "warning"}>{meta?.id ? `v${revision}` : "Draft"}</Badge>
          </div>
        </div>
        <span className="wf-sp" />
        <label className="wf-subject">
          <span>Applies to</span>
          <select
            aria-label="What this workflow runs against"
            value={subject}
            onChange={(e) => setSubject(e.target.value === "board" ? "board" : "ticket")}
          >
            <option value="ticket">Tickets</option>
            <option value="board">Boards (no ticket)</option>
          </select>
        </label>
        <span className="wf-saved">
          {saveState === "saving"
            ? "Saving…"
            : saveState === "saved"
              ? "Saved just now"
              : saveState === "error"
                ? (saveError ?? "Not saved")
                : meta?.id
                  ? `Saved · v${revision}`
                  : "Not saved yet"}
        </span>
        <div role="group" aria-label="View" className="wf-viewswitch">
          <Button variant={view === "canvas" ? "primary" : "ghost"} size="sm" onClick={() => setView("canvas")}>
            Canvas
          </Button>
          <Button variant={view === "steps" ? "primary" : "ghost"} size="sm" onClick={() => setView("steps")}>
            Steps
          </Button>
        </div>
        <Button variant="secondary" size="sm" onClick={runPreflight}>
          Test run
        </Button>
        <Button variant="primary" size="sm" onClick={save} disabled={saveState === "saving"}>
          {meta?.id ? "Save" : "Create workflow"}
        </Button>
      </header>

      {view === "steps" ? (
        <StepsView nodes={nodes} />
      ) : (
        <div className="wf-body" ref={wrapper} onKeyDown={onKeyDown} tabIndex={-1}>
          <ReactFlowProvider>
            <ReactFlow
              nodes={nodes}
              edges={edges}
              nodeTypes={NODE_TYPES}
              onNodesChange={onNodesChange}
              onEdgesChange={onEdgesChange}
              onConnect={onConnect}
              onNodeDragStart={pushHistory}
              onSelectionChange={({ nodes: sel }) => setSelected(sel[0]?.id ?? null)}
              onNodeClick={(_, node) => setSelected(node.id)}
              nodesFocusable
              edgesFocusable
              elementsSelectable
              fitView
              proOptions={{ hideAttribution: true }}
              ariaLabelConfig={{
                "node.a11yDescription.default":
                  "Press Enter to select this step. With it selected, arrow keys move it and Delete removes it.",
                "edge.a11yDescription.default": "Press Enter to select this connection.",
                "controls.ariaLabel": "Canvas controls",
              }}
            >
              <Background variant={BackgroundVariant.Dots} gap={22} size={1.3} color="var(--m-border)" />
              <Controls showInteractive={false} aria-label="Canvas controls" />
              <MiniMap
                className="wf-minimap"
                pannable
                zoomable
                bgColor="var(--m-surface)"
                maskColor="var(--m-surface-2)"
                nodeColor="var(--m-input)"
                ariaLabel="Mini map of the workflow"
              />
              <Panel position="top-left" className="wf-tools">
                <Button variant="primary" size="sm" onClick={() => setLibraryOpen((v) => !v)} aria-expanded={libraryOpen}>
                  Add step
                </Button>
                <Button variant="ghost" size="sm" onClick={undo} disabled={past.current.length === 0} aria-label="Undo">
                  Undo
                </Button>
                <Button variant="ghost" size="sm" onClick={redo} disabled={future.current.length === 0} aria-label="Redo">
                  Redo
                </Button>
                <Button variant="ghost" size="sm" onClick={autoLayout}>
                  Auto layout
                </Button>
              </Panel>
              <Panel position="bottom-center">
                <div className={`wf-strip${errorCount === 0 && issues.length === 0 ? " clean" : ""}`} role="status" aria-live="polite">
                  {issues.length === 0 ? (
                    <>This workflow checks out. No embedded code, every field valid.</>
                  ) : (
                    <>
                      <b>{errorCount > 0 ? `${errorCount} problem${errorCount === 1 ? "" : "s"}` : "Checks out"}</b>
                      <span>— {issues[0]?.message}</span>
                    </>
                  )}
                </div>
              </Panel>
            </ReactFlow>
          </ReactFlowProvider>

          {libraryOpen ? <LibraryPanel palette={palette} onPick={addStep} onClose={() => setLibraryOpen(false)} /> : null}

          <aside className="wf-insp" id="wf-inspector" aria-label="Step settings">
            {selectedNode ? (
              <Inspector
                node={selectedNode}
                registry={byId.get(selectedNode.data.nodeType)}
                issues={issuesByNode.get(selectedNode.id) ?? []}
                allNodes={nodes}
                onParam={updateParams}
                onRemove={removeNode}
                onConnect={connectTo}
              />
            ) : (
              <div className="wf-insp-b">
                <div className="wf-note">
                  Select a step to edit its settings. Every input here is rendered from the step
                  catalogue&apos;s own schema, so it matches what the engine runs.
                </div>
                {dryRun ? (
                  <div className="wf-sec">
                    <h4>Test run</h4>
                    <div className="wf-note">{dryRun.text}</div>
                  </div>
                ) : null}
              </div>
            )}
          </aside>
        </div>
      )}
    </div>
  );
}

// --- steps view (W09) — one document, read as an ordered list ----------------

function StepsView({ nodes }: { nodes: StepNodeType[] }) {
  return (
    <div className="wf-steps">
      <p className="wf-hint wf-steps-lead">
        The same workflow as plain steps. Edit here or on the canvas: it is one document.
      </p>
      {nodes.map((n, i) => (
        <div className="wf-step" key={n.id}>
          <span className="n" aria-hidden="true">
            {i + 1}
          </span>
          <div className="c">
            <b>{n.data.label}</b>
            <div className="s">{FAMILY_LABEL[n.data.family]}</div>
          </div>
        </div>
      ))}
    </div>
  );
}

// --- library / palette -------------------------------------------------------

function LibraryPanel({
  palette,
  onPick,
  onClose,
}: {
  palette: RegistryNodeView[];
  onPick: (reg: RegistryNodeView) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const groups = useMemo(() => {
    const q = query.trim().toLowerCase();
    const filtered = palette.filter(
      (n) => !q || n.label.toLowerCase().includes(q) || n.id.toLowerCase().includes(q),
    );
    const byFamily = new Map<string, RegistryNodeView[]>();
    for (const n of filtered) {
      byFamily.set(n.family, [...(byFamily.get(n.family) ?? []), n]);
    }
    return [...byFamily.entries()];
  }, [palette, query]);

  // Dialog contract: Escape dismisses, and focus returns to whatever held
  // it before the panel opened (the autofocus below moves it in).
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      previous?.focus();
    };
  }, [onClose]);

  return (
    <div className="wf-lib" role="dialog" aria-modal="true" aria-label="Add a step">
      <div className="wf-lib-h">
        <div className="wf-lib-hrow">
          <b className="wf-lib-title">Add a step</b>
          <span className="wf-sp" />
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="Close the step library">
            Close
          </Button>
        </div>
        <input
          className="wf-fi wf-fi-search"
          placeholder="Search steps"
          aria-label="Search steps"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoFocus
        />
      </div>
      <div className="wf-lib-b">
        {groups.map(([family, items]) => (
          <div key={family}>
            <div className="wf-lib-g">{family}</div>
            {items.map((n) => (
              <button key={n.id} type="button" className="wf-lib-it" onClick={() => onPick(n)}>
                <span className="wfnd-i" aria-hidden="true">
                  {family.slice(0, 1).toUpperCase()}
                </span>
                <span>
                  <b>{n.label}</b>
                  <span>{n.agent_description.slice(0, 64)}</span>
                </span>
              </button>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}

// --- inspector (schema-driven) ----------------------------------------------

function Inspector({
  node,
  registry,
  issues,
  allNodes,
  onParam,
  onRemove,
  onConnect,
}: {
  node: StepNodeType;
  registry: RegistryNodeView | undefined;
  issues: string[];
  allNodes: StepNodeType[];
  onParam: (nodeId: string, key: string, value: unknown) => void;
  onRemove: (nodeId: string) => void;
  onConnect: (from: string, to: string) => void;
}) {
  const fields = useMemo(
    () => schemaFields((registry?.input_schema ?? {}) as JsonSchema),
    [registry],
  );
  return (
    <>
      <div className="wf-insp-h">
        <div>
          <b>{registry?.label ?? node.id}</b>
          <span className="s">{node.id}</span>
        </div>
      </div>
      <div className="wf-insp-b">
        {issues.length > 0 ? (
          <div className="wf-sec">
            <h4>Needs attention</h4>
            <ul className="wf-hint wf-issues">
              {issues.map((m, i) => (
                <li key={i}>{m}</li>
              ))}
            </ul>
          </div>
        ) : null}

        <div className="wf-sec">
          <h4>Settings</h4>
          {fields.length === 0 ? (
            <div className="wf-hint">This step takes no settings.</div>
          ) : (
            fields.map((field) => (
              <div className="wf-f" key={field.key}>
                <label htmlFor={`wf-f-${field.key}`}>
                  {field.label}
                  {field.required ? " *" : ""}
                </label>
                <FieldControl field={field} value={node.data.params[field.key]} onChange={(v) => onParam(node.id, field.key, v)} />
                {field.description ? <div className="wf-hint">{field.description}</div> : null}
              </div>
            ))
          )}
        </div>

        <div className="wf-sec">
          <h4>Connect this step to</h4>
          <select
            className="wf-fi"
            value=""
            aria-label="Connect this step to another step"
            onChange={(e) => {
              if (e.target.value) onConnect(node.id, e.target.value);
            }}
          >
            <option value="">Choose a step…</option>
            {allNodes
              .filter((n) => n.id !== node.id)
              .map((n) => (
                <option key={n.id} value={n.id}>
                  {n.data.label} ({n.id})
                </option>
              ))}
          </select>
          <div className="wf-hint wf-connect-hint">
            A keyboard route to the same connection a drag makes. Use “error” in a label for an
            error branch.
          </div>
        </div>

        <div className="wf-sec">
          <Button variant="destructive" size="sm" onClick={() => onRemove(node.id)}>
            Remove this step
          </Button>
        </div>
      </div>
    </>
  );
}

function FieldControl({
  field,
  value,
  onChange,
}: {
  field: ReturnType<typeof schemaFields>[number];
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  const id = `wf-f-${field.key}`;

  switch (field.kind) {
    case "boolean":
      return (
        <label className="wf-check">
          <input id={id} type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} />
          <span className="wf-hint">{value === true ? "On" : "Off"}</span>
        </label>
      );
    case "enum":
      return (
        <select id={id} className="wf-fi" value={value === undefined ? "" : String(value)} onChange={(e) => onChange(e.target.value)}>
          <option value="">Choose…</option>
          {(field.schema.enum ?? []).map((opt) => (
            <option key={String(opt)} value={String(opt)}>
              {String(opt)}
            </option>
          ))}
        </select>
      );
    case "const":
      return (
        <input id={id} className="wf-fi" value={String(field.schema.const)} readOnly />
      );
    case "integer":
    case "number":
      return (
        <input
          id={id}
          type="number"
          className="wf-fi"
          value={typeof value === "number" ? value : ""}
          onChange={(e) => onChange(e.target.value === "" ? undefined : Number(e.target.value))}
        />
      );
    case "array":
    case "object":
    case "unknown":
      return (
        <textarea
          id={id}
          className="wf-fi wf-fi-area wf-fi-mono"
          value={value === undefined ? "" : JSON.stringify(value, null, 2)}
          placeholder={field.kind === "array" ? "[]" : "{}"}
          onChange={(e) => {
            const raw = e.target.value;
            if (raw.trim() === "") return onChange(undefined);
            try {
              onChange(JSON.parse(raw));
            } catch {
              onChange(raw);
            }
          }}
        />
      );
    case "text":
      return (
        <textarea
          id={id}
          className="wf-fi wf-fi-area"
          value={value === undefined ? "" : String(value)}
          onChange={(e) => onChange(e.target.value)}
        />
      );
    default:
      return (
        <input
          id={id}
          className={field.kind === "uuid" ? "wf-fi wf-fi-mono" : "wf-fi"}
          value={value === undefined ? "" : String(value)}
          onChange={(e) => onChange(e.target.value)}
        />
      );
  }
}
