// @meridian/sdk public surface.
//
// Non-purpose: never imports server crates; generated code is regenerated,
// never hand-edited.
export { MeridianClient } from "./client";
export type {
  MeridianClientOptions,
  MeridianPaths,
  LoginRequest,
  LoginResult,
  MfaRequired,
  RegisterRequest,
  TokenPair,
} from "./client";
export type { paths, webhooks, operations, components } from "./generated/schema";
export type { $defs } from "./generated/schema";
