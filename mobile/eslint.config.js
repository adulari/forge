const expoConfig = require("eslint-config-expo/flat");
const { defineConfig } = require("eslint/config");

module.exports = defineConfig([
  expoConfig,
  {
    // The Emberline design system is built on Reanimated v4: animated state is a
    // `useSharedValue` whose `.value` is mutated directly (often inside effects and
    // worklets). The react-hooks RC ruleset shipped with eslint-config-expo flags every
    // such mutation as `immutability` / `set-state-in-effect` — a false positive for
    // Reanimated's model. Off project-wide so lint reflects real defects, not the
    // mandated animation pattern.
    rules: {
      "react-hooks/immutability": "off",
      "react-hooks/set-state-in-effect": "off",
    },
  },
  {
    // `src-tauri/target` is Cargo's build directory (gitignored). Tauri's build script emits
    // generated JS there — `__global-api-script.js` — which trips `no-unused-expressions`.
    // CI never saw it because it lints a clean `npm ci` checkout, but locally it made
    // `npm run lint` exit 1 with warnings nobody can act on, which trains people to ignore
    // lint output entirely. Generated build artifacts are not source and are not lintable.
    ignores: [
      "dist/*",
      ".expo/*",
      "node_modules/*",
      "public/*",
      "src-tauri/target/**",
      "src-tauri/gen/**",
    ],
  },
]);
