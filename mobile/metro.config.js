const path = require("path");

const { getDefaultConfig } = require("expo/metro-config");

const config = getDefaultConfig(__dirname);

// The OTA publish (`eas update` → `expo export`) runs on a memory-constrained
// self-hosted laptop runner. Metro's default (~cpus-1) transform workers each
// fork a full Node process; their combined peak RAM during bundling exhausts the
// machine and freezes it, so GitHub loses the runner and fails the job at ~600s
// with empty logs. METRO_MAX_WORKERS (set in eas-update.yml) caps that fan-out.
// Unset locally, so developer machines keep full parallelism.
const maxWorkers = process.env.METRO_MAX_WORKERS;
if (maxWorkers) {
  config.maxWorkers = Number(maxWorkers);
}

// Metro's file watcher walks everything under the project root, which includes the Tauri
// crate's build directory. A concurrent `cargo build` (desktop work, or `tauri dev`) writes
// and unlinks temp artifacts there faster than the watcher can stat them, and the resulting
// ENOENT kills the whole dev server. Nothing under `src-tauri/target` is ever bundled, so
// exclude it from both the watcher and the resolver.
//
// `blockList` is APPENDED TO, not rebuilt through `exclusionList()`. Expo's default is already a
// finished array of RegExps — one of which is `/^(?:android[\\/]app[\\/]build|…)$/` — and
// `exclusionList`'s `escapeRegExp` rewrites `\/` to `\` + `path.sep` in every pattern's source. On
// Windows that turns the `[\\/]` separator classes into `[\\\]`, an unterminated character class,
// so requiring this file threw `SyntaxError: Invalid regular expression` and every Windows desktop
// build failed in `expo export`. On Linux `path.sep` is `/`, the rewrite is a no-op, and the bug is
// invisible — which is exactly why it shipped.
const escapedTauriTarget = path
  .join(__dirname, "src-tauri", "target")
  .replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const previousBlockList = config.resolver.blockList;
config.resolver.blockList = [
  ...(Array.isArray(previousBlockList)
    ? previousBlockList
    : previousBlockList
      ? [previousBlockList]
      : []),
  // Both separators: `path.join` yields `\` on Windows and `/` elsewhere, and nothing rewrites
  // this pattern for us now.
  new RegExp(`^${escapedTauriTarget}[\\\\/].*`),
];

module.exports = config;
