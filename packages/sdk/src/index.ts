// @marula-core/sdk public surface.
//
// Non-purpose: never imports server crates; generated code is regenerated,
// never hand-edited.
export { MarulaClient, snapshotOf, MarulaApiError } from "./client";
export type {
  MarulaClientOptions,
  MarulaPaths,
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
