// Mirror of `is_noise_path` in crates/forge-cli/src/serve_git.rs. Applied to untracked rows only,
// so a daemon that predates the server-side filter still gets a clean change queue.
const NOISE_DIRS = new Set(["__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", "node_modules"]);
const NOISE_SUFFIXES = [".pyc", ".pyo", ".swp"];

export function isNoisePath(path: string): boolean {
  const segments = path.split("/").filter((segment) => segment.length > 0);
  const name = segments.pop();
  if (name === undefined) return false;
  if (segments.some((segment) => NOISE_DIRS.has(segment))) return true;
  return name === ".DS_Store" || NOISE_SUFFIXES.some((suffix) => name.endsWith(suffix));
}
