// @marula-core/sdk public surface.
//
// Non-purpose: never imports server crates; generated code is regenerated,
// never hand-edited.
export { MeridianClient, snapshotOf, MeridianApiError } from "./client";
export type {
  MeridianClientOptions,
  MeridianPaths,
  LoginRequest,
  LoginResult,
  MfaRequired,
  RegisterRequest,
  TokenPair,
  EntitySnapshot,
  MutationMetaView,
} from "./client";
export type { paths, webhooks, operations, components } from "./generated/schema";
export type { $defs } from "./generated/schema";
