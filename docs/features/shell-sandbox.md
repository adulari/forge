# Feature: OS-level shell sandbox (Linux Landlock)

> Status: **SHIPPED**. Opt-in, default off.

When `[shell] sandbox = true`, every `shell` tool command runs under a kernel-enforced
[Landlock](https://docs.kernel.org/userspace-api/landlock.html) ruleset that confines filesystem
**writes** to the workspace (the command's cwd), the system temp dir, and any extra paths in
`sandbox_writable`. Reads + execute stay broad so normal tooling (compilers, interpreters, package
managers) still works. This is a real, kernel-enforced boundary — unlike the model-side permission
broker, the process *cannot* write outside the allowed set regardless of what the model decides.

```toml
[shell]
sandbox = true
sandbox_writable = ["/home/me/.cargo"]   # extra writable dirs beyond cwd + temp
```

## How it works

- The sandbox is applied via `pre_exec` on the spawned shell process: in the forked child, just
  before `exec`, Forge installs the Landlock ruleset (read+execute on `/`, read+write on the
  writable set) with `CompatLevel::BestEffort` so partial-ABI kernels still get partial protection.
- Kernel support is probed **once in the parent** before spawning. If Landlock is unavailable
  (old kernel, non-Linux), Forge logs a one-time warning and runs the command **unconfined** —
  the sandbox never hard-fails or blocks a command.
- **Linux-only.** On macOS/Windows the whole path is a compile-time no-op.

## Scope + limits

- Confines **filesystem writes**, not network (network egress restriction via Landlock ABI v4 is a
  possible follow-up). Pair with the permission broker + denylist for command-level control.
- Applies to the in-process `shell` tool on the primary session path. Default off so existing
  behaviour is byte-for-byte unchanged.
- **PTY interaction:** the `shell` tool's `pty: true` path (pseudo-terminal) is spawned by
  `portable-pty`, which exposes no `pre_exec` hook, so Landlock can't confine it. To prevent a
  trivial escape (always passing `pty: true`), `pty: true` is **refused** while the sandbox is
  enabled — the model must re-run without PTY.

Verified on Linux 6.x/7.x (`CONFIG_SECURITY_LANDLOCK=y`): a write inside cwd succeeds, a write to
`/etc/...` is denied with `Permission denied`.

## Scoped cargo target dir (`shell.scoped_cargo_target`)

An autonomous/bypass-mode agent that dogfoods Forge on a Rust workspace needs to run
`cargo check`/`build` to verify its own edits. Under confinement — Forge's own Landlock sandbox
or an outer container that mounts the checkout read-only — cargo cannot create `<workspace>/target`
and fails with `Read-only file system (os error 30)` on the build lock.

Enabling `shell.scoped_cargo_target = true` makes the `shell` tool inject a writable
`CARGO_TARGET_DIR` for recognized cargo commands, pointing at a per-project subdir of
`shell.scoped_cargo_target_dir` (default `<system-temp>/forge-cargo-target`). The build's target
tree lands there instead of the read-only workspace, so the compile-check succeeds. The scoped dir
is also folded into the Landlock writable set, so it works when `shell.sandbox` is on too.

- **Opt-in, default off** — normal runs are byte-for-byte unchanged.
- **Only recognized cargo commands** get the env var; other commands are untouched.
- **An explicit `CARGO_TARGET_DIR` in the environment always wins** (the caller's choice is
  respected).
- Independent of `shell.sandbox`: it also helps under an outer container that confines writes
  without Forge's Landlock.
- Confinement is **not** weakened for arbitrary writes — only the build-target dir is carved out.
