// /settings/plugins — import drawers (ADR 0009 sources as adapters).
//
// Purpose: GitHub-repo pull (owner/repo/TAG — pinning is REQUIRED, HEAD/
// main/master/latest are never accepted, exact 40-hex commit pinned with
// it) and the local zip sandbox import with a live validation preview
// (caps shown in the dropzone: 8 MiB archive, ≤256 entries, ≤4 MB per
// member; the client refuses oversized archives before the wire, the
// server re-validates authoritatively). Both land in the staged queue.
//
// Non-purpose: no direct install from these drawers — member uploads
// always ride the staged/approval pipeline.
"use client";

import { Button, Dialog, DialogActions, DialogContent, Input, Label, Spinner, Tag } from "@marula-core/ui";
import { useState } from "react";
import { api, uploadPluginZip } from "../../../lib/api";
import { reasonCopy } from "../../../lib/error-copy";

const ZIP_ARCHIVE_CAP = 8 * 1024 * 1024;
const NON_PINS = new Set(["head", "main", "master", "latest"]);

export function GithubImportDrawer({
  open,
  onOpenChange,
  onStaged,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onStaged: () => void;
}) {
  const [owner, setOwner] = useState("");
  const [repo, setRepo] = useState("");
  const [tag, setTag] = useState("");
  const [commit, setCommit] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const tagProblem =
    tag.trim() === ""
      ? "Tag is required — repos without a pinned tag are refused."
      : NON_PINS.has(tag.trim().toLowerCase())
        ? `“${tag.trim()}” is a floating ref — use a version tag (v1.2.3), never HEAD/main/latest.`
        : null;
  const commitProblem =
    commit.trim() === ""
      ? "Exact commit (40 hex chars) is required — the manifest is pulled at the commit."
      : /^[0-9a-f]{40}$/i.test(commit.trim())
        ? null
        : "The commit must be the full 40-hex sha.";

  const submit = async () => {
    setBusy(true);
    setError(null);
    const { error } = await api_Github(owner, repo, tag, commit);
    setBusy(false);
    if (error !== undefined) {
      const envelopeReason =
        (error as { error?: { reason?: string; message?: string } }).error?.reason ??
        "distribution_manifest_invalid";
      setError(
        `${reasonCopy(envelopeReason)} ${(error as { error?: { message?: string } }).error?.message ?? ""}`.trim(),
      );
      return;
    }
    onOpenChange(false);
    setOwner("");
    setRepo("");
    setTag("");
    setCommit("");
    onStaged();
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        title="Import from a GitHub repository"
        description="Pinned pulls only: a version tag plus the exact commit. Everything imported this way lands PENDING in the staged-approvals queue."
      >
        <div className="grid gap-3 p-4" data-testid="github-import-form">
          <div className="grid grid-cols-2 gap-3">
            <div>
              <Label htmlFor="gh-owner">Owner</Label>
              <Input id="gh-owner" placeholder="acme-corp" value={owner} onChange={(e) => setOwner(e.target.value)} />
            </div>
            <div>
              <Label htmlFor="gh-repo">Repository</Label>
              <Input id="gh-repo" placeholder="marula-collector" value={repo} onChange={(e) => setRepo(e.target.value)} />
            </div>
          </div>
          <div>
            <Label htmlFor="gh-tag">Tag</Label>
            <Input id="gh-tag" placeholder="v1.4.2" value={tag} onChange={(e) => setTag(e.target.value)} />
            {tagProblem !== null ? (
              <p className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="tag-problem">
                {tagProblem}
              </p>
            ) : (
              <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                Pinned pulls only — {Array.from(NON_PINS).slice(0, 4).join(", ")} are refused with distribution_source_not_pinned.
              </p>
            )}
          </div>
          <div>
            <Label htmlFor="gh-commit">Commit (40-hex)</Label>
            <Input id="gh-commit" placeholder="0123456789abcdef0123456789abcdef01234567" value={commit} onChange={(e) => setCommit(e.target.value)} />
            {commitProblem !== null ? (
              <p className="mt-1 text-[12px]" style={{ color: "var(--m-status-danger-fg)" }}>
                {commitProblem}
              </p>
            ) : (
              <p className="mt-1 text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                The manifest is fetched at this exact commit and recorded as tag@sha.
              </p>
            )}
          </div>
          {error !== null ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="github-import-error">
              {error}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="secondary" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button variant="primary" disabled={busy || tagProblem !== null || commitProblem !== null || owner.trim() === "" || repo.trim() === ""} onClick={() => void submit()}>
              {busy ? <Spinner /> : null} Stage import
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}

async function api_Github(owner: string, repo: string, tag: string, commit: string) {
  return api.api.POST("/api/v1/plugins/distribution/staged/github", {
    body: { owner: owner.trim(), repo: repo.trim(), tag: tag.trim(), commit: commit.trim() },
  });
}


/** Counts zip local-file-header signatures — a client-side live estimate
 * for the dropzone preview (the server re-validates authoritatively). */
export function estimateZipEntries(buffer: ArrayBuffer): number | null {
  const bytes = new Uint8Array(buffer);
  const sig = [0x50, 0x4b, 0x03, 0x04];
  let count = 0;
  outer: for (let i = 0; i + 4 <= bytes.length; i += 1) {
    for (let j = 0; j < 4; j += 1) {
      if (bytes[i + j] !== sig[j]) continue outer;
    }
    count += 1;
  }
  return count === 0 ? null : count;
}

export function ZipImportDrawer({
  open,
  onOpenChange,
  onStaged,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onStaged: () => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [entries, setEntries] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const oversize = file !== null && file.size > ZIP_ARCHIVE_CAP;

  const pick = async (picked: File | null) => {
    setFile(picked);
    setEntries(null);
    setError(null);
    if (picked !== null && picked.size <= ZIP_ARCHIVE_CAP) {
      try {
        setEntries(estimateZipEntries(await picked.arrayBuffer()));
      } catch {
        setEntries(null);
      }
    }
  };

  const submit = async () => {
    if (file === null || oversize) return;
    setBusy(true);
    setError(null);
    const result = await uploadPluginZip(file);
    setBusy(false);
    if (result.ok) {
      onOpenChange(false);
      setFile(null);
      onStaged();
      return;
    }
    setError(
      result.status === 413
        ? reasonCopy("distribution_archive_cap")
        : reasonCopy("distribution_archive_invalid"),
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        title="Import a plugin or skill zip"
        description="Sandboxed validation runs before anything registers; a valid archive lands PENDING in the staged-approvals queue."
      >
        <div className="grid gap-3 p-4" data-testid="zip-import-form">
          <div
            className="rounded-[var(--m-radius-container)] border border-dashed p-4 text-center"
            style={{ borderColor: "var(--m-border)" }}
            data-testid="zip-dropzone"
          >
            <p className="text-[13px]" style={{ color: "var(--m-text-secondary)" }}>
              Drop the archive here or
            </p>
            <label className="mt-2 inline-block">
              <span className="m-btn m-btn-secondary" style={{ display: "inline-flex" }}>
                Choose a .zip file
              </span>
              <input
                type="file"
                accept=".zip,application/zip"
                className="sr-only"
                onChange={(e) => void pick(e.target.files?.[0] ?? null)}
                data-testid="zip-file-input"
              />
            </label>
            <p className="mt-2 text-[11px]" style={{ color: "var(--m-text-muted)" }}>
              Sandbox caps: 8 MiB archive · ≤256 entries · ≤4 MB per member · exactly one root manifest.json
            </p>
          </div>
          {file !== null ? (
            <div className="flex flex-wrap items-center gap-2" data-testid="zip-preview">
              <Tag tone={oversize ? "danger" : "neutral"}>{file.name}</Tag>
              <span className="text-[12px]" style={{ color: "var(--m-text-secondary)" }}>
                {Math.ceil(file.size / 1024)} KiB
              </span>
              {entries !== null && !oversize ? (
                entries > 256 ? (
                  <Tag tone="danger">~{entries} entries — exceeds the 256 cap</Tag>
                ) : (
                  <span className="text-[12px]" style={{ color: "var(--m-text-muted)" }}>
                    ~{entries} archive entries
                  </span>
                )
              ) : null}
              {oversize ? (
                <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="zip-cap-error">
                  {reasonCopy("distribution_archive_cap")} Nothing will be imported.
                </p>
              ) : null}
            </div>
          ) : null}
          {error !== null && !oversize ? (
            <p role="alert" className="text-[13px]" style={{ color: "var(--m-status-danger-fg)" }} data-testid="zip-import-error">
              The archive was refused at the sandbox import: {error}
            </p>
          ) : null}
          <DialogActions>
            <Button variant="secondary" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={busy || file === null || oversize || (entries !== null && entries > 256)}
              onClick={() => void submit()}
            >
              {busy ? <Spinner /> : null} Stage import
            </Button>
          </DialogActions>
        </div>
      </DialogContent>
    </Dialog>
  );
}
