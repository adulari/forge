#!/usr/bin/env bash
set -euo pipefail

# Guards scripts/ci/release-targets-guard.sh. Case 1 is the exact shape that broke the v2.13.3
# release, so it is tested first.

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
guard="$script_dir/release-targets-guard.sh"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT

cat > "$work/rust-toolchain.toml" <<'TOML'
[toolchain]
channel = "1.98.0"
components = ["rustfmt", "clippy"]
TOML

write_workflow() {
  # $1 = file, $2 = the `with:` body of the install step (may be empty)
  {
    cat <<'YAML'
name: fixture
jobs:
  build:
    strategy:
      matrix:
        include:
          - target: aarch64-apple-darwin
          - target: x86_64-unknown-linux-gnu
    steps:
      - uses: actions/checkout@v5
      - name: Install Rust
        uses: dtolnay/rust-toolchain@4cda84d5c5c54efe2404f9d843567869ab1699d4
YAML
    if [[ -n $2 ]]; then
      printf '        with:\n'
      printf '%s\n' "$2"
    fi
    printf '      - run: cargo build --target ${{ matrix.target }}\n'
  } > "$1"
}

run_guard() {
  TOOLCHAIN_FILE="$work/rust-toolchain.toml" RELEASE_WORKFLOW="$1" bash "$guard" "$1" 2>&1
}

# 1. The real defect: targets installed without pinning the toolchain the build actually uses.
write_workflow "$work/wf.yml" '          targets: ${{ matrix.target }}'
if output=$(run_guard "$work/wf.yml"); then
  echo 'targets without toolchain must fail' >&2
  exit 1
fi
grep -q 'does not pin' <<<"$output" \
  || { echo "the error must name the missing pin, got: $output" >&2; exit 1; }

# 2. The fix: the channel is derived from rust-toolchain.toml and passed to the action.
write_workflow "$work/wf.yml" '          toolchain: ${{ steps.toolchain.outputs.channel }}
          targets: ${{ matrix.target }}'
run_guard "$work/wf.yml" >/dev/null \
  || { echo 'a derived toolchain pin must pass' >&2; exit 1; }

# 3. `target:` is the action's alias for `targets:` and must be held to the same rule.
write_workflow "$work/wf.yml" '          target: aarch64-apple-darwin'
run_guard "$work/wf.yml" >/dev/null 2>&1 \
  && { echo 'the target alias must fail too' >&2; exit 1; }

# 4. Hard-coding today's channel passes today and silently rots at the next bump.
write_workflow "$work/wf.yml" '          toolchain: "1.98.0"
          targets: ${{ matrix.target }}'
if output=$(run_guard "$work/wf.yml"); then
  echo 'a hard-coded channel must fail' >&2
  exit 1
fi
grep -q 'hard-codes' <<<"$output" \
  || { echo "the error must name the drift, got: $output" >&2; exit 1; }

# 5. An unrelated channel is worse still.
write_workflow "$work/wf.yml" '          toolchain: nightly
          targets: ${{ matrix.target }}'
run_guard "$work/wf.yml" >/dev/null 2>&1 \
  && { echo 'a mismatched channel must fail' >&2; exit 1; }

# 6. A host-only install needs no pin: rustup provisions the toolchain file's channel on demand.
write_workflow "$work/wf.yml" '          components: clippy'
run_guard "$work/wf.yml" >/dev/null \
  || { echo 'a host-only install must pass' >&2; exit 1; }
write_workflow "$work/wf.yml" ''
run_guard "$work/wf.yml" >/dev/null \
  || { echo 'an install with no inputs at all must pass' >&2; exit 1; }

# 7. A target name the pinned toolchain does not know fails before a release run finds out.
cat > "$work/bad-target.yml" <<'YAML'
name: fixture
jobs:
  build:
    strategy:
      matrix:
        include:
          - target: aarch64-apple-darwn
YAML
if output=$(run_guard "$work/bad-target.yml"); then
  if command -v rustup >/dev/null 2>&1; then
    echo 'an unknown target must fail' >&2
    exit 1
  fi
fi

# 8. A toolchain file without a channel is a broken pin, not a pass.
printf '[toolchain]\ncomponents = ["clippy"]\n' > "$work/rust-toolchain.toml"
write_workflow "$work/wf.yml" '          components: clippy'
run_guard "$work/wf.yml" >/dev/null 2>&1 \
  && { echo 'a channel-less toolchain file must fail' >&2; exit 1; }

# 9. The committed workflows are what actually ship; they must satisfy the guard as committed.
bash "$guard" >/dev/null \
  || { echo 'the committed workflows must satisfy the guard' >&2; exit 1; }

echo 'release target guard is enforced'
