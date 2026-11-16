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
  useRequireAuth();
  const router = useRouter();
  const [clients, setClients] = useState<{ id: string; name: string }[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    api.api.GET("/api/v1/clients").then(({ data }) => {
      if (cancelled) return;
      setClients((data ?? []).map((client) => ({ id: client.id as string, name: client.name })));
    });
    return () => {
      cancelled = true;
    };
  }, []);

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
