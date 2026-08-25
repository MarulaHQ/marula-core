/**
 * @module Vitest runner configuration for @meridian/layout.
 *
 * Purpose: run pure-logic assertion tests (serialization, constraint math,
 * tree operations, persistence adapters) in a plain Node environment.
 * Non-purpose: no DOM/component testing here — interaction flows are owned by
 * the repository-level Playwright E2E suite per BUILD-PROMPT §8.
 */
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
  },
});
