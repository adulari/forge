# Kernel confinement — design (spike round 2, part 1)

Status: DRAFT — needs one decision from Floris (§5) before implementation.
Date: 2026-08-07. Follows: `persistent-scripting-tool.md`, `kernel-spike-findings.md`.

## 1. Why this comes before the kernel

Spike round 1 proved the broker mechanism works and proved one thing that reorders the plan:
model-written Python reaches `open()` without ever touching the `forge.*` API, and the prototype
wrote a file the policy had just refused. So the ADR-0008 guarantee cannot rest on the API
surface. Confinement is not a later hardening pass — it is the thing that makes the kernel
shippable at all, and a kernel built first would ship exactly the trust-and-warn posture that
distinguishes Forge from prime-agent today.

## 2. What already exists

`crates/forge-tools/src/sandbox.rs` — `apply_landlock(&writable)`:

- Ruleset handles `AccessFs::from_all(ABI::V5)`, grants `Execute | ReadFile | ReadDir` beneath
  `/`, and full write only beneath each explicitly listed path.
- Applied in `pre_exec` (post-fork, pre-exec), where Landlock syscalls are async-signal-safe.
- `CompatLevel::BestEffort`, and `is_supported()` returns false off Linux.
- Config: `shell.sandbox` (opt-in, default false), `shell.sandbox_writable`,
  plus `shell.scoped_cargo_target` which carves a writable build dir outside a read-only workspace.

So the mechanism, the config vocabulary, and the pre_exec placement are all proven in-tree. The
kernel does not need new sandbox machinery so much as a different **posture**.

## 3. The posture difference, which is the whole point

`shell.rs` applies the ruleset like this, deliberately:

```rust
// Errors are swallowed: a sandbox failure must never prevent the command
// from running (best-effort confinement).
let _ = crate::sandbox::linux::apply_landlock(&writable);
```

Fail-open is defensible for the shell tool: the command was already permitted by the broker, and
confinement is a second belt. **It is not defensible for the kernel**, where confinement is the
only thing between model-authored Python and the filesystem, because the broker is bypassable by
construction (§1). If `apply_landlock` fails for a kernel process, the correct behaviour is to
refuse to start the kernel, not to start it unconfined.

That single difference — fail-closed instead of fail-open — is the core of this design. Concretely:

- `spawn_kernel()` calls `sandbox::is_supported()` **in the parent** before forking. Unsupported →
  the `script` tool is unavailable for this session, with a message saying why. It never silently
  degrades to an unconfined interpreter.
- In `pre_exec`, a failed `apply_landlock` returns `Err` rather than `Ok(())`, so the child dies
  before `exec` instead of running unconfined.
- The failure path gets a test. A guard that has only ever succeeded is not known to work, and
  round 1 already showed the value of proving the deny path (the denied write there was proven,
  not assumed).

## 4. What the ruleset must cover

| Vector | Mechanism | Note |
|---|---|---|
| Filesystem writes | Landlock `AccessFs`, writable set = session scratch + explicitly brokered paths | Same shape as `shell.sandbox_writable`; the workspace is NOT writable by default — writes go through the broker |
| Filesystem reads | `ReadFile \| ReadDir` beneath `/` | Matches the shell tool; a kernel that cannot read the stdlib is useless |
| Spawning processes | Landlock `AccessFs::Execute` is about executing *files*; denying `execve` outright needs **seccomp** | `subprocess`/`os.system` are the obvious bypass and must be closed — this is the one piece with no in-tree precedent |
| Network | Landlock ABI ≥ 4 restricts TCP bind/connect; the current ruleset uses `AccessFs` only | Needs verification against the ABI actually available; UDP and unix sockets are not covered by that mechanism |

The third row is the genuinely new work. Everything else is a re-parameterisation of code that
already ships.

## 5. THE DECISION — non-Linux platforms

Landlock is Linux-only. Forge ships macOS and Windows binaries, so the kernel needs an answer
there, and the three answers differ in what they promise:

**A. Kernel is Linux-only.** `script` simply does not exist on macOS/Windows. Honest, zero risk,
and the guarantee is uniform wherever the feature appears. Cost: a headline capability that is
absent on two of three platforms.

**B. Kernel everywhere, weaker guarantee documented off Linux.** macOS has a sandbox facility
(`sandbox_init`/Seatbelt — long-deprecated by Apple but still functional) and Windows has
AppContainer/job objects. *I have not verified either against this use case*, and saying "sandboxed"
when the enforcement differs per platform is the kind of claim that later turns out to be false.
If B is chosen, each platform needs its own spike before the feature is enabled there.

**C. Kernel everywhere, confinement only on Linux, opt-in elsewhere.** Off Linux the tool exists
but requires an explicit config opt-in that states plainly that side effects are broker-mediated
only, with no OS enforcement behind them. Closest to the current `shell.sandbox` posture — except
that for the shell the broker is authoritative, and here it is not.

Recommendation: **A for the first release, revisit with B after a per-platform spike.** It is the
only option where "the kernel is confined" is true without qualification, and the RFC's own
argument for building this at all was that Forge should not adopt prime-agent's trust-and-warn
model.

## 6. Not decided here

Interpreter choice (wasm-sandboxed vs external `python3`) interacts with this: a wasm runtime
brings its own capability model and could change §4 entirely. That stays in round 2 part 2,
together with criterion 1 (the token/wall-clock measurement), which needs the real harness.
