#!/usr/bin/env bash

# Capture every committed and uncommitted edit relative to a benchmark's
# synthetic base without passing ignored runtime directories to `git add -N`.
capture_git_changes() {
  if [[ $# -lt 4 ]]; then
    echo "usage: capture_git_changes <workspace> <base> <run-dir> <pathspec>..." >&2
    return 2
  fi

  local workspace="$1"
  local base_commit="$2"
  local run_dir="$3"
  shift 3
  local -a patch_paths=("$@")
  local -a untracked_paths=()

  while IFS= read -r -d '' path; do
    untracked_paths+=("$path")
  done < <(
    git -C "$workspace" ls-files --others --exclude-standard -z -- "${patch_paths[@]}"
  )

  if ((${#untracked_paths[@]} > 0)); then
    git -C "$workspace" add -N -- "${untracked_paths[@]}" || return
  fi

  git -C "$workspace" diff --binary "$base_commit" -- "${patch_paths[@]}" \
    >"$run_dir/changes.patch" || return
  git -C "$workspace" status --short -- "${patch_paths[@]}" \
    >"$run_dir/git-status.txt" || return
  git -C "$workspace" diff --check "$base_commit" -- "${patch_paths[@]}" \
    >"$run_dir/git-diff-check.log" 2>&1
}
