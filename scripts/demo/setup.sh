#!/usr/bin/env bash
# Stage a deterministic, fully offline environment for the demo recordings
# (scripts/demo/tapes/*.tape → docs/assets/*.gif):
#
#   .stage/bin/forge     symlink to this repo's release binary (never the installed one)
#   .stage/acme-api      a small scratch git project the demos run in
#   .stage/forge.db      an isolated store (the real ~/.local/share/forge/forge.db is never touched)
#   .stage/env.sh        sourced (hidden) at the top of every tape
#   .stage/tapes/        the templates with @SID@ resolved to the seeded session id
#
# Everything a tape shows is produced by `--mock` (the offline deterministic provider), so
# re-recording needs no API keys and yields the same content every time. `forge mesh` is the one
# exception: it explains routing over whatever provider catalog the recording machine has.
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FORGE="$REPO/target/release/forge"
DEMO="$REPO/scripts/demo"
STAGE="$DEMO/.stage"

[[ -x "$FORGE" ]] || (cd "$REPO" && cargo build --release -p forge-agent)

rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/tapes"
ln -sf "$FORGE" "$STAGE/bin/forge"

# ── the scratch project ──────────────────────────────────────────────────────────────────────
# Std-only on purpose: the autofix pass auto-detects `cargo check`, which must be instant in a
# recording — no dependency downloads mid-GIF.
PROJ="$STAGE/acme-api"
mkdir -p "$PROJ/src" "$PROJ/.forge/workflows"

cat > "$PROJ/Cargo.toml" <<'EOF'
[package]
name = "acme-api"
version = "0.3.1"
edition = "2021"

# The stage lives inside the Forge repo checkout; stay out of its cargo workspace.
[workspace]
EOF

cat > "$PROJ/src/main.rs" <<'EOF'
mod auth;
mod routes;

use std::net::TcpListener;

fn main() {
    let listener = TcpListener::bind("127.0.0.1:8080").expect("bind");
    for stream in listener.incoming().flatten() {
        routes::handle(stream);
    }
}
EOF

cat > "$PROJ/src/auth.rs" <<'EOF'
pub fn bearer_token(header: &str) -> Option<&str> {
    header.strip_prefix("Bearer ")
}
EOF

cat > "$PROJ/src/routes.rs" <<'EOF'
use std::io::Write;
use std::net::TcpStream;

pub fn handle(mut stream: TcpStream) {
    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\nok");
}
EOF

cat > "$PROJ/README.md" <<'EOF'
# acme-api

Small HTTP service with bearer-token auth and a health endpoint.
EOF

# Start recordings in Auto-edit so mock write turns don't stall on an approval prompt.
cat > "$PROJ/.forge/config.toml" <<'EOF'
permission_mode = "accept-edits"
EOF

cat > "$PROJ/.forge/workflows/audit.js" <<'EOF'
await phase("scan");
const areas = ["src/auth.rs", "src/routes.rs", "src/main.rs"];
const findings = await parallel(areas.map((f) => () => agent(`find TODOs and risky patterns in ${f}`)));
await log(`scanned ${areas.length} files`);

await phase("verify");
const confirmed = await pipeline(findings, (prev, f) =>
    agent(`independently verify this finding is real: ${f}`)
);

await phase("report");
await log("assembling the audit report");
return await agent("summarize the confirmed findings in 3 bullets");
EOF

git -C "$PROJ" init -q -b main
git -C "$PROJ" -c user.name=demo -c user.email=demo@example.com add -A
git -C "$PROJ" -c user.name=demo -c user.email=demo@example.com commit -qm "init: acme-api skeleton"

# ── seed the isolated store ──────────────────────────────────────────────────────────────────
# One finished mock session (a write_file turn) so the provenance tape has something to blame,
# replay, and fork; one drained queue task so the hero can show a real autopilot digest.
DB="$STAGE/forge.db"
export FORGE_DB="$DB"

(cd "$PROJ" && "$FORGE" run "create a file" --mock >/dev/null 2>&1)
SID="$(cd "$PROJ" && "$FORGE" sessions 2>/dev/null | awk 'NR==1 {print $1}')"
[[ -n "$SID" ]] || { echo "setup: seeding the provenance session failed" >&2; exit 1; }

(cd "$PROJ" \
  && "$FORGE" queue add "create a file with the v0.4 release notes" --budget 2.50 >/dev/null \
  && "$FORGE" queue run --mock >/dev/null 2>&1)

# Prewarm the autofix lint pass (`cargo check --all-targets`) so it is instant on camera.
(cd "$PROJ" && cargo check --all-targets >/dev/null 2>&1) || true

# Prewarm mesh discovery (catalog + quota probes) so `forge mesh` answers promptly on camera
# instead of showing seconds of blank probing against a cold store.
(cd "$PROJ" && "$FORGE" mesh >/dev/null 2>&1) || true

# ── env sourced (hidden) by every tape ───────────────────────────────────────────────────────
cat > "$STAGE/env.sh" <<EOF
export PATH="$STAGE/bin:\$PATH"
export FORGE_DB="$DB"
export FORGE_MOCK_STREAM_DELAY_MS=80
export SID="$SID"
PS1='\[\e[35m\]❯\[\e[0m\] '
cd "$PROJ"
EOF

# ── resolve tape templates ───────────────────────────────────────────────────────────────────
for tape in "$DEMO"/tapes/*.tape; do
  sed "s/@SID@/$SID/g" "$tape" > "$STAGE/tapes/$(basename "$tape")"
done

echo "staged: $STAGE  (session $SID)"
