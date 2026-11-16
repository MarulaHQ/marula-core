"use client";
/**
 * ClientScopeSelect — the one control these documentation lists share.
 *
 * Purpose: every list in this lane is client-scoped (the brief's first rule:
 * the client is the front door). When a list is opened without a client — from
 * a bookmark or a global expiry link — this control asks which client rather
 * than showing an empty page. The answer travels in the query string, so the
 * URL stays the state and a link can be shared.
 *
 * Non-purpose: not a searchable company picker for 38 clients — those fit in a
 * select. A searchable picker is a separate component on the home page.
 */

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { useRequireAuth, api } from "../../lib/api";

export function ClientScopeSelect({
  clientId,
  label = "Client",
}: {
  clientId: string | null;
  label?: string;
}): React.JSX.Element | null {
  // The list is fetched once, so it must wait for the token restore a full page
  // load performs after mount; a request fired before that carries no bearer.
  const { status } = useRequireAuth();
  const router = useRouter();
  const [clients, setClients] = useState<{ id: string; name: string }[] | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (status !== "authed") return;
    let cancelled = false;
    setFailed(false);
    api.api
      .GET("/api/v1/clients")
      .then(({ data, error }) => {
        if (cancelled) return;
        if (error !== undefined || data === undefined) {
          // An error is not an empty client list: saying "none" would leave the
          // selector empty with no way to retry.
          setClients([]);
          setFailed(true);
          return;
        }
        setClients(data.map((client) => ({ id: client.id as string, name: client.name })));
      })
      .catch(() => {
        // A thrown request is the same fact as an error status.
        if (cancelled) return;
        setClients([]);
        setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, [status, attempt]);

  if (failed) {
    return (
      <span className="flex items-center gap-2 text-[13px]" style={{ color: "var(--m-status-danger-fg)" }}>
        Clients did not load.
        <button
          type="button"
          onClick={() => setAttempt((count) => count + 1)}
          className="h-8 rounded-[8px] border px-2.5 text-[12px] font-semibold"
          style={{ borderColor: "var(--m-border)", color: "var(--m-text)" }}
        >
          Try again
        </button>
      </span>
    );
  }

  if (clients === null) return null;

  return (
    <label className="flex items-center gap-2 text-[13px]">
      <span style={{ color: "var(--m-text-secondary)" }}>{label}</span>
      <select
        value={clientId ?? ""}
        onChange={(event) => {
          const next = event.target.value;
          const url = new URL(window.location.href);
          if (next === "") url.searchParams.delete("client_id");
          else url.searchParams.set("client_id", next);
          router.push(`${url.pathname}${url.search}`);
        }}
        className="h-10 rounded-[10px] border px-2.5 text-[13px]"
        style={{
          borderColor: "var(--m-border)",
          background: "var(--m-surface)",
          color: "var(--m-text)",
        }}
      >
        <option value="">Choose a client…</option>
        {clients.map((client) => (
          <option key={client.id} value={client.id}>
            {client.name}
          </option>
        ))}
      </select>
    </label>
  );
}
