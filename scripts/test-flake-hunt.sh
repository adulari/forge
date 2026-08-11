#!/usr/bin/env bash
set -euo pipefail

# The loop in flake-hunt.sh is trivial; the part that can be silently wrong is extracting WHICH
# tests failed out of cargo's output. If that parse breaks, the script still reports "N runs
# failed" and simply lists nothing — the failure mode looks like success at a glance. So pin it
# against real captured `cargo test` output.

scratch="$(mktemp -d)"
trap 'rm -rf -- "$scratch"' EXIT

# Verbatim shape of a real failing run — this is the ETXTBSY failure that became #1003.
cat >"$scratch/run.log" <<'EOF'
running 64 tests
test registry::tests::live_servers_are_bounded_across_registries ... ok
test server::tests::initialize_rejects_json_rpc_errors ... FAILED

failures:

---- server::tests::initialize_rejects_json_rpc_errors stdout ----

thread 'server::tests::initialize_rejects_json_rpc_errors' panicked at crates/forge-lsp/src/server.rs:650:14:
called `Result::unwrap()` on an `Err` value: Os { code: 26, kind: ExecutableFileBusy }

failures:
    server::tests::initialize_rejects_json_rpc_errors

test result: FAILED. 63 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
EOF

extract() {
  awk '/^failures:$/{f=1;next} /^$/{f=0} f&&/^    /{print $1}' "$1" | sort -u
}

got="$(extract "$scratch/run.log")"
[ "$got" = "server::tests::initialize_rejects_json_rpc_errors" ] || {
  echo "expected the failing test name, got: '$got'" >&2
  exit 1
}

# The stdout block above also contains an indented `thread '...' panicked` line and is introduced
# by a `---- ... stdout ----` header, NOT by a bare `failures:` line. Extracting from that block
# instead of the summary would yield a mangled name; assert we picked exactly one.
[ "$(printf '%s\n' "$got" | wc -l)" -eq 1 ] || {
  echo "expected exactly one name, got: $got" >&2
  exit 1
}

# Several failures in one run must all be reported — a race often takes more than one test with it.
cat >"$scratch/multi.log" <<'EOF'
failures:
    cli::commands::run::driver::tests::mesh_overlay_resolution_reports_a_dirty_frame
    cli::commands::run::driver::tests::steer_input_while_idle_starts_a_turn_immediately

test result: FAILED. 512 passed; 2 failed
EOF
[ "$(extract "$scratch/multi.log" | wc -l)" -eq 2 ] || {
  echo "expected both failing tests" >&2
  exit 1
}

# A clean run must yield nothing, or every green run would be counted as a flake.
cat >"$scratch/pass.log" <<'EOF'
running 64 tests
test server::tests::initialize_rejects_json_rpc_errors ... ok

test result: ok. 64 passed; 0 failed; 0 ignored
EOF
[ -z "$(extract "$scratch/pass.log")" ] || {
  echo "a passing run must report no failing tests" >&2
  exit 1
}

echo "flake-hunt failure extraction matches real cargo output"
