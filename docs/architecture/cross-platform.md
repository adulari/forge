# Cross-platform support (Linux · macOS · Windows)

> **Principle.** Every Forge feature MUST work on Linux, macOS, and Windows. "Works on my
> machine" is not done. A change that only works on one OS is incomplete, the same way a change
> with no tests is incomplete. This is a hard requirement, not an aspiration.

Forge is a developer tool; its users are split across all three platforms, and the whole
value proposition (one harness, BYOK, local-first) collapses if it's Linux-only. Every PR runs the
full Rust suite on the self-hosted Arch pool. A separate scheduled/manual E2E builds Forge and runs
a real model turn plus `forge doctor` on Arch, `windows-latest`, and `macos-latest`; the release
matrix also builds every published OS/architecture target. Cross-platform behavior therefore has a
real gate without spending hosted macOS/Windows capacity on every topic-branch commit.

## What this means in practice

When you add or change a feature, it is not done until:

- It **compiles** on all three targets (the release matrix) and relevant behavior is exercised by
  the cross-platform E2E before release.
- It **behaves correctly** on all three — not just "doesn't crash". Paths, config locations,
  process spawning, terminal handling, and credential storage all differ per OS.
- Any genuinely OS-specific behaviour is **explicit** (`#[cfg(...)]` or a per-OS branch), with
  the other platforms handled — never left to silently misbehave.

## How we keep it portable (the rules)

1. **Never hardcode paths or separators.** No `/home/...`, no `~/`, no `"a/b/c"` string paths.
   Use `std::path::Path`/`PathBuf::join`, and resolve well-known locations through the
   `directories` crate (`BaseDirs::home_dir()` / `config_dir()` / `data_dir()`), which returns
   the correct per-OS location (e.g. config dir = `~/.config` on Linux, `~/Library/Application
   Support` on macOS, `%APPDATA%` on Windows).
2. **Secrets → the OS keyring via the `keyring` crate**, which is wired with native backends
   for all three (`linux-native` Secret Service, `apple-native` Keychain, `windows-native`
   Credential Manager). Never write secrets to files, and never assume a specific backend.
3. **Terminal/TUI → `crossterm` + `ratatui`** only. They abstract the platform differences
   (raw mode, alt-screen, key events) across Unix and the Windows console.
4. **Process spawning → `tokio::process::Command`** with the program and args passed
   separately (no shell string). When you must invoke a shell, branch per-OS
   (`sh -c` vs `cmd /C`) rather than assuming POSIX.
5. **Async/IO → tokio + reqwest with the `rustls` stack** (no OpenSSL C dependency), so TLS
   and networking build identically everywhere.
6. **No Unix-only syscalls** in cross-cutting code. If a feature needs `libc`/`unix` APIs,
   gate them with `#[cfg(unix)]` and provide a Windows path.
7. **Line endings & encoding:** don't assume `\n`-only or a specific filesystem encoding when
   parsing tool output or files.

## Guarded platform seams

| Area | Current implementation | Regression gate |
|------|------------------------|-----------------|
| Shell tool (`forge-tools::shell`) | `sh -c` on Unix, `cmd /C` on Windows; timeouts kill the Unix process group or Windows tree (`taskkill /T`) | Unit tests plus the three-OS real-turn workflow |
| CLI bridges (`claude-cli` / `codex-cli`) | Windows PATH lookup tries `.exe`/`.cmd`/`.bat`; npm shims launch through quoted `cmd /S /C` | Windows-specific resolver/quoting tests plus the three-OS E2E |
| Linux release ABI | x86-64 and ARM64 build inside a digest-pinned Debian Bullseye container, not against rolling Arch libraries | glibc 2.31, GLIBCXX 3.4.28, and no-ALSA dependency gate plus distro Docker battery |

The **MCP client** (this subsystem) is portable by construction: source discovery uses
`directories::BaseDirs` (per-OS config/home dirs), tokens go to the keyring's native backend,
the import picker uses crossterm, and stdio servers spawn via `tokio::process` — no hardcoded
paths or Unix-only calls.

## Checklist for a new feature

- [ ] No hardcoded paths/separators; well-known dirs via `directories`.
- [ ] Secrets via `keyring`, never files.
- [ ] Any shell/process invocation is portable or per-OS branched.
- [ ] Terminal interaction via crossterm/ratatui.
- [ ] `#[cfg]`-gate anything genuinely OS-specific; handle every platform.
- [ ] The Arch PR gates are green; run the manual three-OS E2E before release when the change
      touches paths, process launching, terminal behavior, keyrings, bridges, or OS integration.
