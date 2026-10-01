// The setup wizard's one shared helper.
//
// Purpose: an invite's one-time setup token, read out of the mutation
// response. Both wizards hand this link to a person, and the link is shown
// exactly once — the server cannot show it again — so the copy that carries it
// has to say so.
//
// Non-purpose: nothing else lives here. The step bodies belong to the pages,
// because each step owns its own fields and its own endpoint.
"use client";

import type { components } from "@marula-core/sdk";

/**
 * The one-time setup token from an invite response, or null when the server
 * did not return one (which happens when the response shape changes, so the
 * caller shows a sentence rather than an empty code block).
 */
export function inviteSetupToken(
  data: components["schemas"]["OrgUserMutationResponse"] | undefined,
): string | null {
  const candidate = data as unknown as
    | { user?: { setup_token?: string }; setup_token?: string }
    | undefined;
  return candidate?.user?.setup_token ?? candidate?.setup_token ?? null;
}
