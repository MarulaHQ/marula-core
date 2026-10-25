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
      // Device storage: the suite exercises the real prefixing and sweep
      // against an in-memory stand-in instead of the native module.
      "@react-native-async-storage/async-storage": path.join(
        here,
        "tests",
        "async-storage-stub.ts",
      ),
    },
  },
  test: {
    environment: "node",
    include: ["tests/**/*.test.ts"],
  },
});
