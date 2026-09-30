import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vitest/config";

const here = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  resolve: {
    alias: {
      "@": path.join(here, "src"),
      // The pack-rules tests read tokens.ts, which imports React Native for
      // the theme hook. The node suite never renders; stub the module.
      "react-native": path.join(here, "tests", "rn-stub.ts"),
    },
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
  },
});
