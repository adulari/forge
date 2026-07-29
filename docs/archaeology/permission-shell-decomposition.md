# Code archaeology: shell command decomposition

## Boundary

`forge-core/src/permission/shell_commands.rs` owns one question: *what does this shell command
line actually run?* It unwraps statement separators (`;`, `&&`, `||`, `|`, `&`, newline), heredocs,
command substitution (`$(…)`, backticks), `bash -c` / `cmd /C` wrappers, and no-op wrapper
binaries, and reports whether the scan completed. The broker keeps every decision: mode
precedence, the builtin deny floor, rule specificity, per-segment allow/ask, and path matching.

## Interface

`effective_commands(cmd) -> (Vec<String>, bool)` is the only item the broker imports
(`pub(super)`). The boolean is load-bearing: `false` means the scan gave up part-way, which widens
the broker's literal-substring fallback. Everything else in the module stays private.

## Why the split is safe

The extraction is a pure move — `git show HEAD:permission.rs` diffed against the new module shows
exactly one changed line, the visibility of `effective_commands`. No parsing rule, depth limit,
scan cap, or `ok` transition was touched.

## Characterization

The broker's security tests (builtin deny floor, newline/heredoc/substitution evasion, the fuzz
invariant test) continue to exercise this module through `effective_commands` and all pass
unchanged. The `split_operators` quoting/newline test moved with the code it characterizes.

## Standing caveat

The SECURITY NOTE moved intact with the code: this is an approximation, not a shell parser. The
denylist is a floor against accidents and lazy attacks; real containment is `shell.sandbox` or a
container.
