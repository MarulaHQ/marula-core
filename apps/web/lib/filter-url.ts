/**
 * The filter model's shareable link.
 *
 * One encoding for the state the builder edits: the named view, the match mode
 * and the clauses. A link that reproduces the rows is the pack's requirement
 * (DESIGN-NOTES-tickets.md §6), and it has to survive a copy into chat, so the
 * clauses ride one query parameter rather than a JSON blob.
 */
export interface SharedFilterState {
  view?: string;
  match?: "all" | "any";
  filters?: { field: string; op: string; value?: string }[];
}

function encodeClause(clause: { field: string; op: string; value?: string }): string {
  return [clause.field, clause.op, clause.value ?? ""].map(encodeURIComponent).join("~");
}

/** Build the URL for the current state, over the given base (origin + path). */
export function filterUrl(
  base: string,
  state: {
    view?: string;
    match?: "all" | "any";
    filters: { field: string; op: string; value?: string }[];
    sort?: { field: string; dir: "asc" | "desc" };
  },
): string {
  const url = new URL(base);
  url.search = "";
  if (state.view !== undefined && state.view !== "") url.searchParams.set("view", state.view);
  if (state.match === "any") url.searchParams.set("match", "any");
  if (state.filters.length > 0) {
    url.searchParams.set("f", state.filters.map(encodeClause).join(";"));
  }
  if (state.sort !== undefined) {
    url.searchParams.set("sort", `${state.sort.field}:${state.sort.dir}`);
  }
  return url.toString();
}

/** Read a shared link's state. Unknown or malformed parts are dropped. */
export function parseFilterUrl(search: string): SharedFilterState {
  const params = new URLSearchParams(search);
  const out: SharedFilterState = {};
  const view = params.get("view");
  if (view !== null && view !== "") out.view = view;
  const match = params.get("match");
  if (match === "all" || match === "any") out.match = match;
  const encoded = params.get("f");
  if (encoded !== null && encoded !== "") {
    const filters = encoded
      .split(";")
      .map((part): { field: string; op: string; value?: string } | null => {
        const [field, op, value] = part.split("~").map((piece) => decodeURIComponent(piece));
        if (field === undefined || field === "" || op === undefined || op === "") return null;
        return { field, op, value: value === "" || value === undefined ? undefined : value };
      })
      .filter((clause): clause is { field: string; op: string; value?: string } => clause !== null);
    if (filters.length > 0) out.filters = filters;
  }
  return out;
}
