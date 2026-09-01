#!/usr/bin/env bash
set -euo pipefail

# Guard the contract between rust-toolchain.toml and every workflow that cross-installs a Rust
# target.
#
# dtolnay/rust-toolchain does not read rust-toolchain.toml: its `toolchain` input defaults to
# `stable`. A step that passes `targets:` without `toolchain:` therefore installs that target's
# std for stable, while cargo — which does obey the toolchain file — builds with the pinned
# channel, for which only the host target was ever installed. The build then fails with
# `error[E0463]: can't find crate for std`. That is how v2.13.3's x86_64-apple-darwin release leg
# broke: release.yml is workflow_dispatch-only, so no PR ever exercised it.
#
# Checks:
#   1. every dtolnay/rust-toolchain step that installs a target also pins `toolchain:`
#   2. that pin is derived from rust-toolchain.toml, not a hard-coded channel that can drift
#   3. every release matrix target is a real target of the pinned toolchain
#
# Usage: release-targets-guard.sh [workflow ...]   (defaults to the repository's workflows)

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
toolchain_file=${TOOLCHAIN_FILE:-$repo_root/rust-toolchain.toml}
release_workflow=${RELEASE_WORKFLOW:-$repo_root/.github/workflows/release.yml}

failed=0
fail() {
  printf 'guard failed: %s\n' "$1" >&2
  failed=1
}

if [[ ! -f $toolchain_file ]]; then
  fail "no toolchain file at $toolchain_file"
  exit 1
fi

channel=$(sed -n 's/^ *channel *= *"\(.*\)"/\1/p' "$toolchain_file" | head -n1)
if [[ -z $channel ]]; then
  fail "$toolchain_file declares no [toolchain] channel"
  exit 1
fi

workflows=("$@")
if ((${#workflows[@]} == 0)); then
  while IFS= read -r path; do
    workflows+=("$path")
  done < <(find "$repo_root/.github/workflows" -name '*.yml' | sort)
fi

# Walk each `uses: dtolnay/rust-toolchain@...` step and report the inputs of its `with:` block.
# Emits one `file:line:targets:toolchain` record per step, where each field is `-` when absent.
scan_steps() {
  awk '
    function flush() {
      if (in_step) {
        printf "%s:%d:%s:%s\n", FILENAME, step_line, (has_targets ? targets : "-"), (has_toolchain ? toolchain : "-")
        in_step = 0
      }
    }
    {
      line = $0
      sub(/[[:space:]]+$/, "", line)
      if (line ~ /^[[:space:]]*#/ || line == "") next
      match(line, /^[[:space:]]*/)
      indent = RLENGTH
      key = line
      sub(/^[[:space:]]*-?[[:space:]]*/, "", key)

      # A step ends at the next list item at or above its indent, or at any shallower line. Keys
      # of the same step can sit at the `uses:` indent, so equal indent alone does not end it.
      is_item = (line ~ /^[[:space:]]*-[[:space:]]/)
      if (in_step && (indent < step_indent || (is_item && indent <= step_indent))) flush()

      if (key ~ /^uses:[[:space:]]*dtolnay\/rust-toolchain@/) {
        flush()
        in_step = 1
        step_indent = indent
        step_line = FNR
        has_targets = 0
        has_toolchain = 0
        next
      }
      if (!in_step) next
      if (key ~ /^(targets|target):/) {
        has_targets = 1
        targets = key
        sub(/^[^:]*:[[:space:]]*/, "", targets)
      }
      if (key ~ /^toolchain:/) {
        has_toolchain = 1
        toolchain = key
        sub(/^[^:]*:[[:space:]]*/, "", toolchain)
      }
    }
    END { flush() }
  ' "$@"
}

while IFS= read -r record; do
  [[ -n $record ]] || continue
  file=${record%%:*}
  rest=${record#*:}
  line=${rest%%:*}
  rest=${rest#*:}
  targets=${rest%%:*}
  toolchain=${rest#*:}
  toolchain=$(sed -e 's/^"\(.*\)"$/\1/' -e "s/^'\\(.*\\)'\$/\\1/" <<<"$toolchain")
  where="${file#"$repo_root/"}:$line"

  # A step that installs no extra target uses only the host std, which rustup provisions for the
  # toolchain file's channel on demand. Those steps are unaffected.
  [[ $targets != "-" ]] || continue
  # An expression that resolves to the empty string installs nothing for that matrix leg, but the
  # same step serves legs where it does not, so it still has to pin.
  [[ $targets != "''" && $targets != '""' ]] || continue

  if [[ $toolchain == "-" ]]; then
    fail "$where installs targets ($targets) but does not pin \`toolchain:\`, so they are installed for \`stable\` while cargo builds with $channel from rust-toolchain.toml"
    continue
  fi
  case "$toolchain" in
    *rust-toolchain*|*steps.toolchain.outputs.channel*) ;;
    "$channel")
      fail "$where hard-codes toolchain: $channel; derive it from rust-toolchain.toml so a channel bump cannot leave it behind"
      ;;
    *)
      fail "$where pins toolchain: $toolchain, which is neither derived from rust-toolchain.toml nor its channel ($channel)"
      ;;
  esac
done < <(scan_steps "${workflows[@]}")

# Release matrix targets must exist for the pinned channel. `rustup target list` names every
# target the channel can install, so a typo or a target dropped from a Rust release fails here
# rather than mid-release.
if [[ -f $release_workflow ]]; then
  mapfile -t release_targets < <(sed -n 's/^ *-\? *target: *\([a-z0-9_]*-[a-z0-9_.-]*\) *$/\1/p' "$release_workflow" | sort -u)
  if ((${#release_targets[@]} == 0)); then
    fail "found no build targets in ${release_workflow#"$repo_root/"}"
  elif command -v rustup >/dev/null 2>&1; then
    if ! known=$(rustup target list --toolchain "$channel" 2>/dev/null); then
      fail "rustup cannot list targets for the pinned toolchain $channel"
    else
      for target in "${release_targets[@]}"; do
        grep -qx -e "$target" -e "$target (installed)" <<<"$known" ||
          fail "release target $target is not a target of the pinned toolchain $channel"
      done
    fi
  else
    printf 'note: rustup unavailable, skipping target resolution for %s\n' "$channel"
  fi
fi

if ((failed)); then
  exit 1
fi

printf 'release target guard: %s targets resolve against pinned toolchain %s\n' \
  "${#release_targets[@]}" "$channel"
