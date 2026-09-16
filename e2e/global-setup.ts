// Global setup: wait for core-api health + a seeded demo login before any
// test runs. Fails fast with a clear message when the environment is missing
// instead of cascading 401s through every test.
import { DEMO_PASSWORD, DEMO_OWNER_EMAIL, apiLogin, waitForApi } from "./helpers/api";

export default async function globalSetup() {
  await waitForApi();
  const tokens = await apiLogin(DEMO_OWNER_EMAIL, DEMO_PASSWORD);
  if (!tokens.access_token) throw new Error("demo owner login returned no access_token");
  console.log(`[e2e] demo env ready — owner login OK (expires_in=${tokens.expires_in}s)`);
}
