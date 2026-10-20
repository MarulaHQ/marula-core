// Wave 47 lane A25 — the static half of the fix, run from the unit suite.
//
// `scripts/check-command-targets.ts` imports the real command registry and
// resolves every control a command clicks against the source tree, per
// surface. This suite calls the same functions, so a rename of
// `data-testid="new-record"` fails `pnpm --filter web test` in the commit that
// made it. The middle test is the historical case exactly: the Wave 41 pack
// port renamed the /clients button while /boards kept its testid, so a check
// that only asked "does this testid exist anywhere?" would have stayed green.
import { describe, expect, it } from "vitest";
import { COMMAND_REGISTRY } from "../../../lib/commands/registry";
import { auditTargets, loadedSources, selfTest } from "../../../../../scripts/check-command-targets";

describe("every command target resolves against the source tree", () => {
  it("reports no misses on the shipped registry", () => {
    const audit = auditTargets();
    expect(audit.violations).toEqual([]);
    expect(audit.targets).toBeGreaterThan(0);
  });

  it("catches a rename on ONE surface while another keeps the testid", () => {
    const files = loadedSources();
    const renamed = files.map((file) =>
      file.path === "apps/web/app/clients/page.tsx"
        ? { ...file, text: file.text.replace('data-testid="new-record"', 'data-testid="new-client"') }
        : file,
    );
    const audit = auditTargets(COMMAND_REGISTRY, renamed);

    const clients = audit.violations.find((v) => v.command === "new.clients");
    expect(clients?.selector).toBe('[data-testid="new-record"]');
    expect(clients?.reason).toContain("/clients");
    // /boards still renders its own new-record, so new.ticket is untouched.
    expect(audit.violations.some((v) => v.command === "new.ticket")).toBe(false);
  });

  it("self-test: the checker notices when any declared control is renamed", () => {
    const { checked, missed } = selfTest();
    expect(missed).toEqual([]);
    expect(checked).toBeGreaterThan(0);
  });
});
