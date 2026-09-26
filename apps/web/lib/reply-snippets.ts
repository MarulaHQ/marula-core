// Reply-snippet library client (Wave 34 lane 3, `p34-canned-templates`).
//
// Purpose: typed access to the staff canned-response library for the
// composer picker and the /settings/reply-snippets manager. Types mirror
// the OpenAPI wire structs; the generated SDK owns the transport.

"use client";

import { api } from "./api";

export interface ReplySnippet {
  id: string;
  name: string;
  body: string;
  /** Boards the snippet is offered on. Empty = every board. */
  board_ids: string[];
  /** Fork-on-edit provenance: the origin snippet, when a fork. */
  forked_from: string | null;
  updated_by: string;
  updated_at: string;
}

export interface ReplySnippetsListResponse {
  snippets: ReplySnippet[];
  /** The closed ticket-variable vocabulary snippets may use. */
  vocabulary: string[];
}

/** Lists the org's snippets with the search-in-composer predicate. */
export async function listReplySnippets(params: {
  board_id?: string;
  q?: string;
}): Promise<ReplySnippetsListResponse> {
  const { data, error } = await api.api.GET("/api/v1/reply-snippets", {
    params: { query: params },
  });
  if (error !== undefined || data === undefined) {
    throw new Error("The snippet library could not be loaded.");
  }
  return data as ReplySnippetsListResponse;
}

/** Renders a snippet's variable tokens against a ticket context. */
export async function renderReplySnippet(id: string, contextId: string): Promise<string> {
  const { data, error } = await api.api.POST("/api/v1/reply-snippets/{id}/render", {
    params: { path: { id } },
    body: { context_id: contextId },
  });
  if (error !== undefined || data === undefined) {
    throw new Error("The snippet could not be resolved for this ticket.");
  }
  return data.body;
}

/** Creates a snippet (admin). Rides the IF1 envelope. */
export async function createReplySnippet(body: {
  name: string;
  body: string;
  board_ids: string[];
  idempotency_key: string;
}): Promise<ReplySnippet> {
  const { data, error } = await api.api.POST("/api/v1/reply-snippets", { body });
  if (error !== undefined || data === undefined) {
    throw new Error("The snippet could not be saved.");
  }
  return data as ReplySnippet;
}

/** Edits a snippet in place (admin). Rides the IF1 envelope. */
export async function updateReplySnippet(
  id: string,
  body: { name?: string; body?: string; board_ids?: string[]; idempotency_key: string },
): Promise<ReplySnippet> {
  const { data, error } = await api.api.PATCH("/api/v1/reply-snippets/{id}", {
    params: { path: { id } },
    body,
  });
  if (error !== undefined || data === undefined) {
    throw new Error("The snippet could not be saved.");
  }
  return data as ReplySnippet;
}

/** Forks a snippet into a new editable copy (admin, fork-on-edit). */
export async function forkReplySnippet(id: string, name: string): Promise<ReplySnippet> {
  const { data, error } = await api.api.POST("/api/v1/reply-snippets/{id}/fork", {
    params: { path: { id } },
    body: { name, idempotency_key: `fork-${crypto.randomUUID()}` },
  });
  if (error !== undefined || data === undefined) {
    throw new Error("The snippet could not be duplicated.");
  }
  return data as ReplySnippet;
}

/** Deletes a snippet (admin). */
export async function deleteReplySnippet(id: string): Promise<void> {
  const { error } = await api.api.DELETE("/api/v1/reply-snippets/{id}", {
    params: { path: { id } },
  });
  if (error !== undefined) {
    throw new Error("The snippet could not be removed.");
  }
}
