#!/usr/bin/env bash
set -euo pipefail

# Drives scripts/ci/npm-audit-gate.sh against synthetic `npm audit --json` reports, so the gate
# is proven to BLOCK as well as to pass. A waiver nobody can see failing is not a gate.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
gate="$script_dir/npm-audit-gate.sh"
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

# The gate shells out to `npm audit`; stub it so these cases are hermetic and offline.
make_npm() {
  local report=$1
  mkdir -p "$scratch/bin"
  cat > "$scratch/bin/npm" <<STUB
#!/usr/bin/env bash
cat "$report"
exit 1
STUB
  chmod +x "$scratch/bin/npm"
}

run_gate() {
  make_npm "$1"
  PATH="$scratch/bin:$PATH" bash "$gate"
}

cat > "$scratch/waived.json" <<'JSON'
{"vulnerabilities": {
  "image-size": {"severity": "high", "range": "*", "fixAvailable": true,
    "via": [{"url": "https://github.com/advisories/GHSA-w3rx-r6r6-pgpr"},
            {"url": "https://github.com/advisories/GHSA-5p2g-fcmc-qvqq"}]},
  "metro": {"severity": "high", "range": ">=0.22.1", "fixAvailable": true, "via": ["image-size"]}
}}
JSON
if ! run_gate "$scratch/waived.json" >/dev/null; then
  echo "expected the allowlisted image-size advisories to pass" >&2
  exit 1
fi

cat > "$scratch/blocking.json" <<'JSON'
{"vulnerabilities": {
  "somelib": {"severity": "critical", "range": "<2.0.0", "fixAvailable": true,
    "via": [{"url": "https://github.com/advisories/GHSA-aaaa-bbbb-cccc"}]}
}}
JSON
if run_gate "$scratch/blocking.json" >/dev/null 2>&1; then
  echo "expected an unwaived critical advisory to fail the gate" >&2
  exit 1
fi

# The waiver is justified ONLY while nothing is upgradable. A bounded range means upstream
# published a fix, so the entry must stop passing rather than outlive its reason.
cat > "$scratch/expired.json" <<'JSON'
{"vulnerabilities": {
  "image-size": {"severity": "high", "range": "<2.0.3", "fixAvailable": true,
    "via": [{"url": "https://github.com/advisories/GHSA-w3rx-r6r6-pgpr"}]}
}}
JSON
if run_gate "$scratch/expired.json" >/dev/null 2>&1; then
  echo "expected an allowlisted advisory with a bounded range to fail the gate" >&2
  exit 1
fi

# Moderate and low findings are reported by npm but must not gate a release.
cat > "$scratch/moderate.json" <<'JSON'
{"vulnerabilities": {
  "smalllib": {"severity": "moderate", "range": "<1.2.3", "fixAvailable": true,
    "via": [{"url": "https://github.com/advisories/GHSA-dddd-eeee-ffff"}]}
}}
JSON
if ! run_gate "$scratch/moderate.json" >/dev/null; then
  echo "expected a moderate advisory not to gate" >&2
  exit 1
fi

# A fix that exists only in a major a dependency pins out of the tree is waived while every
# installed copy is below that major, and fails once one reaches it.
cat > "$scratch/zod.json" <<'JSON'
{"vulnerabilities": {
  "zod": {"severity": "high", "range": "<4.6.6", "fixAvailable": false,
    "via": [{"url": "https://github.com/advisories/GHSA-mrhx-c7xc-m3w5"}]}
}}
JSON
mkdir -p "$scratch/pinned" "$scratch/moved"
echo '{"packages": {"node_modules/zod": {"version": "3.25.76"}}}' > "$scratch/pinned/package-lock.json"
echo '{"packages": {"node_modules/zod": {"version": "4.2.0"}, "node_modules/x/node_modules/zod": {"version": "3.25.76"}}}' > "$scratch/moved/package-lock.json"
if ! (cd "$scratch/pinned" && run_gate "$scratch/zod.json" >/dev/null); then
  echo "expected a major-locked advisory to pass while the tree is pinned below the fix" >&2
  exit 1
fi
if (cd "$scratch/moved" && run_gate "$scratch/zod.json" >/dev/null 2>&1); then
  echo "expected a major-locked advisory to fail once the fixed major is installed" >&2
  exit 1
fi

echo "npm audit gate behaviour passed"
