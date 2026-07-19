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

module.exports = config;
