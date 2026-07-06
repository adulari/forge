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
    ignores: ["dist/*", ".expo/*", "node_modules/*"],
  },
]);
