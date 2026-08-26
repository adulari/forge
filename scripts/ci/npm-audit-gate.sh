#!/usr/bin/env bash
set -euo pipefail

# Fail on any high/critical npm advisory in the SHIPPED dependency tree, except a small,
# explicitly justified allowlist.
#
# Why this exists rather than plain `npm audit --audit-level=high`: that command has no
# per-advisory ignore. A single unfixable transitive advisory therefore blocks every mobile
# and protocol PR indefinitely, which is what happened here — `image-size` reports
# `first_patched_version: null` on both of its advisories and the registry's newest published
# version is itself in the vulnerable range, so there is nothing to upgrade to, for us or for
# upstream Expo/Metro. The Rust side already carries the same shape of exception
# (`cargo audit --ignore RUSTSEC-...`), so this keeps the two ecosystems consistent.
#
# An allowlisted id still fails the gate once a fixed version exists, so the waiver cannot
# silently outlive its justification. The expiry signal is the advisory's VULNERABLE RANGE,
# not npm's `fixAvailable` flag: for image-size npm reports `fixAvailable: true` while the
# range is `*` and its own text form admits the "fix" is a breaking react-native downgrade.
# A range of `*` means every published version is affected and there is nothing to upgrade
# to; the moment upstream publishes a fixed version the range becomes bounded and this gate
# starts failing until the waiver is removed.

allowed_ids=(
  # image-size: ICNS parser infinite loop. No patched version exists at any version.
  # Reached only through the bundler/CLI (expo -> @expo/cli -> @expo/metro -> metro ->
  # image-size), so realistic exposure is a malformed image hanging a build, on images from
  # this repository. Not reachable from app runtime code: no app source imports it.
  GHSA-w3rx-r6r6-pgpr
  # image-size: JXL and HEIF parser infinite loops. Same package, same path, same reasoning.
  GHSA-5p2g-fcmc-qvqq
)

audit_json=$(npm audit --omit=dev --json 2>/dev/null || true)
if [ -z "$audit_json" ]; then
  echo "npm-audit-gate: npm audit produced no output" >&2
  exit 1
fi

printf '%s' "$audit_json" | ALLOWED="${allowed_ids[*]}" python3 -c '
import json, os, sys

allowed = set(os.environ["ALLOWED"].split())
try:
    report = json.load(sys.stdin)
except json.JSONDecodeError as error:
    print(f"npm-audit-gate: could not parse npm audit output: {error}", file=sys.stderr)
    raise SystemExit(1)

blocking = []
waived = []
for name, vuln in sorted(report.get("vulnerabilities", {}).items()):
    if vuln.get("severity") not in ("high", "critical"):
        continue
    ids = {
        via.get("url", "").rsplit("/", 1)[-1]
        for via in vuln.get("via", [])
        if isinstance(via, dict)
    }
    ids.discard("")
    # A package with no advisory ids of its own is vulnerable only because a dependency is;
    # it clears as soon as that root advisory does, so judge it by its roots.
    if not ids:
        continue
    unwaived = ids - allowed
    if unwaived:
        blocking.append((name, vuln["severity"], sorted(unwaived)))
    else:
        waived.append((name, sorted(ids), vuln.get("range", "")))

for name, ids, affected_range in waived:
    if affected_range.strip() != "*":
        blocking.append((name, "waived-but-now-fixable", ids))
        print(
            f"npm-audit-gate: {name} {ids} is allowlisted, but its affected range is now "
            f"{affected_range!r} rather than '*' — a fixed version exists, so upgrade it and "
            "drop the entry from scripts/ci/npm-audit-gate.sh",
            file=sys.stderr,
        )
    else:
        print(
            f"npm-audit-gate: waived {name} {ids} "
            "(affected range is '*' — no fixed version published)"
        )

if blocking:
    print("", file=sys.stderr)
    for name, severity, ids in blocking:
        print(f"npm-audit-gate: BLOCKING {name} ({severity}) {ids}", file=sys.stderr)
    print(
        "\nnpm-audit-gate: fix these, or add a justified entry to "
        "scripts/ci/npm-audit-gate.sh if genuinely unfixable.",
        file=sys.stderr,
    )
    raise SystemExit(1)

print("npm-audit-gate: no unwaived high or critical advisories in the shipped tree")
'
