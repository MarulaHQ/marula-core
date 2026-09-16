"use client";
/**
 * Saved article prose. Presentational component for the deprecated body
 * text contract is NOT included — server-sanitized content_html renders
 * as-is (the sanitizer is core-kb's whitelist, re-rendered per-tenant
 * tokens via the .m-editor-canvas/.m-kb-prose vocabulary). Editors don't
 * ship inside this component.
 */
import { cn } from "../lib/cn";

export function KbProse({
  className,
  html,
}: {
  className?: string;
  html: string;
}) {
  return (
    <div
      className={cn("m-editor-canvas m-kb-prose", className)}
      // The payload is the server-side sanitized renderer's output,
      // never user HTML (ADR 0007: script/javascript:/unsanctioned
      // iframes cannot serialize — see crates/core-kb/src/render.rs).
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
