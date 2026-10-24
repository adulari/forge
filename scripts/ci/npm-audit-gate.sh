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
  # braces: stack exhaustion on deeply nested patterns. Affected range is '*' (3.0.3 is the newest
  # published). Reached only via expo -> @expo/cli -> @expo/metro-file-map -> micromatch -> braces,
  # i.e. the bundler/CLI globbing repository paths at build time. Not imported by app runtime code.
  # Recheck 2027-01-11.
  GHSA-vfj7-8cjw-p6xm
  # node-forge: RSA PKCS#1 v1.5 signature verification flaw. Affected range is '*' (1.4.0 is the
  # newest published). Reached only via expo-updates -> @expo/code-signing-certificates and
  # @expo/cli, i.e. the build/publish-time code-signing CLI; the app bundle never imports it.
  # Recheck 2027-01-11.
  GHSA-86w9-cpqp-85rv
)

# Advisories fixed only in a newer MAJOR that a dependency's pin keeps out of the tree. The range
# is bounded, so the `*` rule above would reject them, yet there is nothing we can upgrade to.
# Format: ID=package@fixed-major. The waiver lapses as soon as any installed copy of the package
# reaches that major: the pin moved, so upgrade to the fix.
major_locked=(
  # zod: memory exhaustion validating oversized constrained collections. Fixed only in 4.6.6;
  # no 3.x patch exists, and @expo/cli pins zod ^3.25.76. Reached via @expo/cli (validates its own
  # build config), babel-plugin-react-compiler and eslint-plugin-react-hooks; the app bundle never
  # imports zod. Recheck when Expo moves to zod 4.
  GHSA-mrhx-c7xc-m3w5=zod@4
)

audit_json=$(npm audit --omit=dev --json 2>/dev/null || true)
if [ -z "$audit_json" ]; then
  echo "npm-audit-gate: npm audit produced no output" >&2
  exit 1
fi

printf '%s' "$audit_json" | ALLOWED="${allowed_ids[*]}" MAJOR_LOCKED="${major_locked[*]}" python3 -c '
import json, os, sys

allowed = set(os.environ["ALLOWED"].split())
major_locked = {}
for entry in os.environ.get("MAJOR_LOCKED", "").split():
    advisory, _, target = entry.partition("=")
    package, _, major = target.rpartition("@")
    major_locked[advisory] = (package, int(major))

def installed_majors(package):
    with open("package-lock.json") as handle:
        packages = json.load(handle).get("packages", {})
    return {
        int(meta["version"].split(".")[0])
        for path, meta in packages.items()
        if path.split("node_modules/")[-1] == package and "version" in meta
    }
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
    locked = set()
    for advisory in ids & set(major_locked):
        package, fixed_major = major_locked[advisory]
        majors = installed_majors(package)
        if majors and max(majors) < fixed_major:
            locked.add(advisory)
            print(f"npm-audit-gate: waived {name} [{advisory!r}] (fix needs {package} {fixed_major}.x; tree is pinned to {sorted(majors)})")
        else:
            print(f"npm-audit-gate: {advisory} no longer major-locked: {package} {sorted(majors)} installed; upgrade to the fix and drop the entry", file=sys.stderr)
    ids = ids - locked
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
